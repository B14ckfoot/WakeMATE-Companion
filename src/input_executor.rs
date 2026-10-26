//! One bounded, owned input thread. No HTTP runtime worker emits OS input.
use crate::input::{self, Button, InputBackend, InputEvent, InputKey, ValidatedInput};
pub use crate::input::{InputCommand, InputError};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use tracing::warn;

pub type AuthorizationCheck = Arc<dyn Fn() -> bool + Send + Sync + 'static>;
const QUEUE_CAPACITY: usize = 32;
const WATCHDOG_TICK: Duration = Duration::from_millis(20);
const HOLD_LEASE: Duration = Duration::from_secs(2);
const TEXT_BATCH_CHARS: usize = 16;

#[derive(Default)]
struct Generations {
    global: u64,
    devices: HashMap<String, u64>,
}
#[derive(Default)]
struct Control {
    stopped: AtomicBool,
    generations: Mutex<Generations>,
}

#[derive(Clone)]
struct Ticket {
    owner: String,
    global: u64,
    device: u64,
    control: Arc<Control>,
    authorize: AuthorizationCheck,
}
impl Ticket {
    fn check(&self) -> Result<(), InputError> {
        if self.control.stopped.load(Ordering::Acquire) {
            return Err(InputError::Cancelled);
        }
        {
            let state = self
                .control
                .generations
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if state.global != self.global
                || state.devices.get(&self.owner).copied().unwrap_or(0) != self.device
            {
                return Err(InputError::Cancelled);
            }
        }
        // A stale HTTP request may enqueue after a revoke incremented the
        // generation. Live authorization therefore remains mandatory as well.
        if !std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (self.authorize)()))
            .unwrap_or(false)
        {
            return Err(InputError::Denied);
        }
        Ok(())
    }
}

struct Request {
    command: ValidatedInput,
    ticket: Ticket,
    reply: oneshot::Sender<Result<(), InputError>>,
}
struct Inner {
    sender: SyncSender<Request>,
    control: Arc<Control>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.control.stopped.store(true, Ordering::Release);
        if let Some(join) = self.join.lock().unwrap_or_else(|p| p.into_inner()).take() {
            if join.join().is_err() {
                warn!("remote input worker stopped unexpectedly");
            }
        }
    }
}

#[derive(Clone)]
pub struct InputExecutor {
    inner: Arc<Inner>,
}

impl InputExecutor {
    pub fn new() -> Result<Self, InputError> {
        Self::with_backend(input::platform_backend(), QUEUE_CAPACITY, HOLD_LEASE)
    }

    fn with_backend(
        backend: Box<dyn InputBackend>,
        capacity: usize,
        hold_lease: Duration,
    ) -> Result<Self, InputError> {
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let control = Arc::new(Control::default());
        let worker_control = control.clone();
        let join = thread::Builder::new()
            .name("WakeMATE input".into())
            .spawn(move || {
                run_worker(receiver, worker_control, backend, hold_lease);
            })
            .map_err(|_| InputError::Unavailable)?;
        Ok(Self {
            inner: Arc::new(Inner {
                sender,
                control,
                join: Mutex::new(Some(join)),
            }),
        })
    }

    pub async fn execute(
        &self,
        owner: &str,
        command: InputCommand,
        authorize: AuthorizationCheck,
    ) -> Result<(), InputError> {
        self.submit(owner, command, authorize)?
            .await
            .map_err(|_| InputError::Unavailable)?
    }

    fn submit(
        &self,
        owner: &str,
        command: InputCommand,
        authorize: AuthorizationCheck,
    ) -> Result<oneshot::Receiver<Result<(), InputError>>, InputError> {
        if owner.is_empty() || owner.len() > 128 {
            return Err(InputError::Denied);
        }
        let command = command.validate()?;
        let ticket = {
            let state = self
                .inner
                .control
                .generations
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            Ticket {
                owner: owner.into(),
                global: state.global,
                device: state.devices.get(owner).copied().unwrap_or(0),
                control: self.inner.control.clone(),
                authorize,
            }
        };
        ticket.check()?;
        let (reply, receiver) = oneshot::channel();
        match self.inner.sender.try_send(Request {
            command,
            ticket,
            reply,
        }) {
            Ok(()) => Ok(receiver),
            Err(TrySendError::Full(_)) => Err(InputError::Busy),
            Err(TrySendError::Disconnected(_)) => Err(InputError::Unavailable),
        }
    }

    /// Invalidates queued/in-flight work; the worker releases remote-held
    /// buttons on its next short batch or watchdog tick.
    pub fn cancel_device(&self, owner: &str) {
        let mut state = self
            .inner
            .control
            .generations
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let generation = state.devices.entry(owner.into()).or_default();
        *generation = generation.wrapping_add(1);
    }

    pub fn cancel_all(&self) {
        let mut state = self
            .inner
            .control
            .generations
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        state.global = state.global.wrapping_add(1);
    }

    /// Permanent shutdown also refuses requests admitted by an HTTP handler
    /// that outlives listener cancellation. A restart needs a new executor.
    pub fn shutdown(&self) {
        self.inner.control.stopped.store(true, Ordering::Release);
    }
}

struct Hold {
    ticket: Ticket,
    expires: Instant,
}
struct Worker {
    backend: Box<dyn InputBackend>,
    buttons: Vec<(Button, Hold)>,
    keys: Vec<InputKey>,
    hold_lease: Duration,
    recovery: bool,
}

impl Worker {
    fn record(&mut self, events: &[InputEvent], ticket: &Ticket) {
        for event in events {
            match *event {
                InputEvent::Key(key, true) => {
                    if !self.keys.contains(&key) {
                        self.keys.push(key);
                    }
                }
                InputEvent::Key(key, false) => self.keys.retain(|held| *held != key),
                InputEvent::Button(button, true) => {
                    self.buttons.retain(|(held, _)| *held != button);
                    self.buttons.push((
                        button,
                        Hold {
                            ticket: ticket.clone(),
                            expires: Instant::now() + self.hold_lease,
                        },
                    ));
                }
                InputEvent::Button(button, false) => {
                    self.buttons.retain(|(held, _)| *held != button)
                }
                _ => {}
            }
        }
    }

    fn send(&mut self, events: &[InputEvent], ticket: &Ticket) -> Result<(), InputError> {
        let owned_buttons: Vec<_> = self.buttons.iter().map(|(button, _)| *button).collect();
        if self
            .backend
            .local_input_busy(events, &owned_buttons, &self.keys)
        {
            return Err(InputError::Busy);
        }
        match self.backend.send(events) {
            Ok(()) => {
                self.record(events, ticket);
                Ok(())
            }
            Err(error) => {
                self.record(&events[..error.accepted.min(events.len())], ticket);
                self.recovery = true;
                self.release_all();
                Err(InputError::Execution(error.detail))
            }
        }
    }

    /// Release retries preserve any events Windows still refuses. They never
    /// inject key-ups for keys/buttons this worker did not press.
    fn release_events(&mut self, events: &[InputEvent]) {
        if events.is_empty() {
            return;
        }
        let accepted = match self.backend.send(events) {
            Ok(()) => events.len(),
            Err(error) => error.accepted.min(events.len()),
        };
        for event in &events[..accepted] {
            match *event {
                InputEvent::Key(key, false) => self.keys.retain(|held| *held != key),
                InputEvent::Button(button, false) => {
                    self.buttons.retain(|(held, _)| *held != button)
                }
                _ => {}
            }
        }
        if accepted != events.len() {
            self.recovery = true;
        }
    }

    fn release_all(&mut self) {
        let mut events: Vec<_> = self
            .keys
            .iter()
            .rev()
            .map(|key| InputEvent::Key(*key, false))
            .collect();
        events.extend(
            self.buttons
                .iter()
                .map(|(button, _)| InputEvent::Button(*button, false)),
        );
        self.release_events(&events);
        self.recovery = !self.keys.is_empty() || !self.buttons.is_empty();
    }

    fn release_owner(&mut self, owner: &str) {
        let events: Vec<_> = self
            .buttons
            .iter()
            .filter(|(_, hold)| hold.ticket.owner == owner)
            .map(|(button, _)| InputEvent::Button(*button, false))
            .collect();
        self.release_events(&events);
    }

    fn tick(&mut self) {
        if self.recovery {
            self.release_all();
            return;
        }
        let now = Instant::now();
        let events: Vec<_> = self
            .buttons
            .iter()
            .filter(|(_, hold)| hold.expires <= now || hold.ticket.check().is_err())
            .map(|(button, _)| InputEvent::Button(*button, false))
            .collect();
        self.release_events(&events);
    }

    fn check(request: &Request) -> Result<(), InputError> {
        if request.reply.is_closed() {
            return Err(InputError::Cancelled);
        }
        request.ticket.check()
    }

    fn process(&mut self, request: &Request) -> Result<(), InputError> {
        Self::check(request)?;
        if !self.backend.is_supported() {
            return Err(InputError::Unsupported(
                "remote input is supported only by the Windows companion".into(),
            ));
        }
        self.tick();
        if self.recovery {
            return Err(InputError::Execution(
                "Windows has not yet released prior remote input; remote input is paused".into(),
            ));
        }
        if self
            .buttons
            .iter()
            .any(|(_, hold)| hold.ticket.owner != request.ticket.owner)
        {
            return Err(InputError::Busy);
        }
        let events = match &request.command {
            ValidatedInput::Move(x, y) => vec![InputEvent::Move(*x, *y)],
            ValidatedInput::Scroll(amount) => vec![InputEvent::Scroll(*amount)],
            ValidatedInput::Click(button, double) => {
                if self.buttons.iter().any(|(held, _)| held == button) {
                    return Err(InputError::Busy);
                }
                let mut events = vec![
                    InputEvent::Button(*button, true),
                    InputEvent::Button(*button, false),
                ];
                if *double {
                    events.extend_from_within(..);
                }
                events
            }
            ValidatedInput::Button(button, down) => {
                let held = self.buttons.iter().any(|(held, _)| held == button);
                if held == *down {
                    Vec::new()
                } else {
                    vec![InputEvent::Button(*button, *down)]
                }
            }
            ValidatedInput::Shortcut(shortcut) => {
                input::shortcut_events(shortcut, self.backend.as_mut())?
            }
            ValidatedInput::Text(text) => {
                let mut chars = text.chars().peekable();
                while chars.peek().is_some() {
                    Self::check(request)?;
                    let mut events = Vec::with_capacity(TEXT_BATCH_CHARS * 4);
                    for character in chars.by_ref().take(TEXT_BATCH_CHARS) {
                        let mut buffer = [0_u16; 2];
                        for unit in character.encode_utf16(&mut buffer).iter() {
                            events.push(InputEvent::Key(InputKey::Unicode(*unit), true));
                            events.push(InputEvent::Key(InputKey::Unicode(*unit), false));
                        }
                    }
                    self.send(&events, &request.ticket)?;
                    thread::yield_now();
                }
                Vec::new()
            }
        };
        Self::check(request)?;
        if !events.is_empty() {
            self.send(&events, &request.ticket)?;
        }
        for (_, hold) in &mut self.buttons {
            if hold.ticket.owner == request.ticket.owner {
                hold.expires = Instant::now() + self.hold_lease;
            }
        }
        Ok(())
    }
}

fn run_worker(
    receiver: Receiver<Request>,
    control: Arc<Control>,
    backend: Box<dyn InputBackend>,
    hold_lease: Duration,
) {
    let mut worker = Worker {
        backend,
        buttons: Vec::new(),
        keys: Vec::new(),
        hold_lease,
        recovery: false,
    };
    while !control.stopped.load(Ordering::Acquire) {
        worker.tick();
        match receiver.recv_timeout(WATCHDOG_TICK) {
            Ok(request) => {
                let result = worker.process(&request);
                if matches!(result, Err(InputError::Cancelled | InputError::Denied)) {
                    worker.release_owner(&request.ticket.owner);
                }
                let _ = request.reply.send(result);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    worker.release_all();
    // A secure-desktop transition can temporarily reject release events. Bound
    // shutdown retries and retain an honest warning instead of hanging the UI.
    for _ in 0..4 {
        if !worker.recovery {
            break;
        }
        thread::sleep(Duration::from_millis(5));
        worker.release_all();
    }
    if worker.recovery {
        warn!("Windows refused remote input cleanup; a local button/key release may be required");
    }
    while let Ok(request) = receiver.try_recv() {
        let _ = request.reply.send(Err(InputError::Cancelled));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        input::BackendFailure,
        types::{MouseButtonAction, MouseButtonArg},
    };
    use std::sync::Condvar;

    type BatchLog = Arc<Mutex<Vec<(Vec<InputEvent>, usize)>>>;
    type Gate = Arc<(Mutex<bool>, Condvar)>;
    struct Pause {
        entered: SyncSender<()>,
        gate: Gate,
    }
    #[derive(Default)]
    struct FakeState {
        batches: BatchLog,
        local_busy: AtomicBool,
        pause: Mutex<Option<Pause>>,
        fail: Mutex<Option<usize>>,
        after_send: Mutex<Option<AuthorizationCheck>>,
    }
    struct Fake(Arc<FakeState>);
    impl InputBackend for Fake {
        fn local_input_busy(&mut self, _: &[InputEvent], _: &[Button], _: &[InputKey]) -> bool {
            self.0.local_busy.load(Ordering::Acquire)
        }
        fn send(&mut self, events: &[InputEvent]) -> Result<(), BackendFailure> {
            let failure = self.0.fail.lock().unwrap().take();
            let accepted = failure.unwrap_or(events.len()).min(events.len());
            self.0
                .batches
                .lock()
                .unwrap()
                .push((events.to_vec(), accepted));
            let pause = self.0.pause.lock().unwrap().take();
            if let Some(pause) = pause {
                let _ = pause.entered.send(());
                let (mutex, condition) = &*pause.gate;
                let mut open = mutex.lock().unwrap();
                while !*open {
                    open = condition.wait(open).unwrap();
                }
            }
            if let Some(after) = self.0.after_send.lock().unwrap().take() {
                after();
            }
            if failure.is_some() {
                Err(BackendFailure {
                    accepted,
                    detail: "fake OS rejected input".into(),
                })
            } else {
                Ok(())
            }
        }
        fn character_key(&mut self, c: char) -> Result<(u16, Vec<u16>), InputError> {
            if c.is_ascii_alphabetic() {
                Ok((
                    c.to_ascii_uppercase() as u16,
                    if c.is_ascii_uppercase() {
                        vec![0x10]
                    } else {
                        vec![]
                    },
                ))
            } else {
                Err(InputError::Unsupported(
                    "fake layout cannot map this key".into(),
                ))
            }
        }
    }
    fn executor(capacity: usize, lease: Duration) -> (InputExecutor, Arc<FakeState>) {
        let fake = Arc::new(FakeState::default());
        (
            InputExecutor::with_backend(Box::new(Fake(fake.clone())), capacity, lease).unwrap(),
            fake,
        )
    }
    fn allow() -> AuthorizationCheck {
        Arc::new(|| true)
    }
    fn movement() -> InputCommand {
        InputCommand::MouseMove {
            delta_x: 1,
            delta_y: 2,
        }
    }
    fn down() -> InputCommand {
        InputCommand::MouseButton {
            button: MouseButtonArg::Left,
            action: MouseButtonAction::Down,
        }
    }
    fn up() -> InputCommand {
        InputCommand::MouseButton {
            button: MouseButtonArg::Left,
            action: MouseButtonAction::Up,
        }
    }
    fn set_pause(fake: &FakeState) -> (Receiver<()>, Gate) {
        let (entered, receiver) = mpsc::sync_channel(1);
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        *fake.pause.lock().unwrap() = Some(Pause {
            entered,
            gate: gate.clone(),
        });
        (receiver, gate)
    }
    fn open(gate: &Gate) {
        let (mutex, condition) = &**gate;
        *mutex.lock().unwrap() = true;
        condition.notify_all();
    }
    fn accepted(fake: &FakeState) -> Vec<InputEvent> {
        fake.batches
            .lock()
            .unwrap()
            .iter()
            .flat_map(|(events, count)| events[..*count].iter().copied())
            .collect()
    }
    async fn wait_for_release(fake: &FakeState) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while !accepted(fake).contains(&InputEvent::Button(Button::Left, false)) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("worker should release the remote-held button");
    }

    #[tokio::test]
    async fn bounded_queue_rejects_overload_without_running_it() {
        let (executor, fake) = executor(2, HOLD_LEASE);
        let (entered, gate) = set_pause(&fake);
        let first = executor.submit("a", movement(), allow()).unwrap();
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let second = executor.submit("a", movement(), allow()).unwrap();
        let third = executor.submit("a", movement(), allow()).unwrap();
        let rejected = executor.submit("a", movement(), allow());
        open(&gate); // Open before asserting so a failed assertion cannot hang Drop.
        assert!(matches!(rejected, Err(InputError::Busy)));
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        third.await.unwrap().unwrap();
        assert_eq!(accepted(&fake).len(), 3);
    }

    #[tokio::test]
    async fn complete_shortcuts_are_serial_batches_with_balanced_modifiers() {
        let (executor, fake) = executor(4, HOLD_LEASE);
        let first = executor
            .submit(
                "a",
                InputCommand::KeyPress {
                    key: "ctrl+c".into(),
                },
                allow(),
            )
            .unwrap();
        let second = executor
            .submit(
                "b",
                InputCommand::KeyPress {
                    key: "shift+v".into(),
                },
                allow(),
            )
            .unwrap();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        let batches = fake.batches.lock().unwrap();
        assert_eq!(batches.len(), 2);
        for (index, (modifier, key)) in [(0x11, 0x43), (0x10, 0x56)].into_iter().enumerate() {
            assert_eq!(
                batches[index].0,
                vec![
                    InputEvent::Key(InputKey::Virtual(modifier), true),
                    InputEvent::Key(InputKey::Virtual(key), true),
                    InputEvent::Key(InputKey::Virtual(key), false),
                    InputEvent::Key(InputKey::Virtual(modifier), false)
                ]
            );
        }
    }

    #[tokio::test]
    async fn queued_work_is_cancelled_and_late_stale_authorization_is_denied() {
        let (executor, fake) = executor(4, HOLD_LEASE);
        let (entered, gate) = set_pause(&fake);
        let first = executor.submit("a", movement(), allow()).unwrap();
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let queued = executor
            .submit(
                "a",
                InputCommand::TextInput {
                    text: "never type".into(),
                },
                allow(),
            )
            .unwrap();
        executor.cancel_device("a");
        open(&gate);
        let _ = first.await.unwrap();
        assert!(matches!(queued.await.unwrap(), Err(InputError::Cancelled)));
        assert!(matches!(
            executor.execute("a", movement(), Arc::new(|| false)).await,
            Err(InputError::Denied)
        ));
        assert_eq!(accepted(&fake), vec![InputEvent::Move(1, 2)]);
    }

    #[tokio::test]
    async fn live_revocation_stops_text_after_one_short_batch() {
        let (executor, fake) = executor(4, HOLD_LEASE);
        let permitted = Arc::new(AtomicBool::new(true));
        let revoke = permitted.clone();
        *fake.after_send.lock().unwrap() = Some(Arc::new(move || {
            revoke.store(false, Ordering::Release);
            true
        }));
        let auth: AuthorizationCheck = Arc::new(move || permitted.load(Ordering::Acquire));
        let result = executor
            .execute(
                "a",
                InputCommand::TextInput {
                    text: "x".repeat(256),
                },
                auth,
            )
            .await;
        assert!(matches!(result, Err(InputError::Denied)));
        assert_eq!(accepted(&fake).len(), TEXT_BATCH_CHARS * 2);
    }

    #[tokio::test]
    async fn disconnect_cancels_inflight_text_and_queued_work() {
        let (executor, fake) = executor(4, HOLD_LEASE);
        let (entered, gate) = set_pause(&fake);
        let request = executor
            .submit(
                "a",
                InputCommand::TextInput {
                    text: "x".repeat(256),
                },
                allow(),
            )
            .unwrap();
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let queued = executor.submit("a", movement(), allow()).unwrap();
        drop(request);
        drop(queued);
        open(&gate);
        executor.execute("b", movement(), allow()).await.unwrap();
        assert_eq!(accepted(&fake).len(), TEXT_BATCH_CHARS * 2 + 1);
    }

    #[tokio::test]
    async fn button_ownership_and_cancel_release_remote_state() {
        let (executor, fake) = executor(4, HOLD_LEASE);
        executor.execute("a", down(), allow()).await.unwrap();
        assert!(matches!(
            executor.execute("b", up(), allow()).await,
            Err(InputError::Busy)
        ));
        assert_eq!(
            accepted(&fake),
            vec![InputEvent::Button(Button::Left, true)]
        );
        executor.cancel_device("a");
        wait_for_release(&fake).await;
        executor.execute("b", up(), allow()).await.unwrap();
        assert_eq!(
            accepted(&fake).len(),
            2,
            "an unowned up must not release a local button"
        );
    }

    #[tokio::test]
    async fn watchdog_pause_and_shutdown_all_release_held_buttons() {
        let (watchdog, fake) = executor(4, Duration::from_millis(35));
        watchdog.execute("a", down(), allow()).await.unwrap();
        wait_for_release(&fake).await;
        let (paused, fake) = executor(4, HOLD_LEASE);
        paused.execute("a", down(), allow()).await.unwrap();
        paused.cancel_all();
        wait_for_release(&fake).await;
        let (stopped, fake) = executor(4, HOLD_LEASE);
        stopped.execute("a", down(), allow()).await.unwrap();
        drop(stopped);
        assert_eq!(
            accepted(&fake),
            vec![
                InputEvent::Button(Button::Left, true),
                InputEvent::Button(Button::Left, false)
            ]
        );
    }

    #[tokio::test]
    async fn permanent_shutdown_rejects_late_admission_and_releases_state() {
        let (executor, fake) = executor(4, HOLD_LEASE);
        executor.execute("a", down(), allow()).await.unwrap();
        executor.shutdown();
        assert!(matches!(
            executor.execute("a", movement(), allow()).await,
            Err(InputError::Cancelled)
        ));
        wait_for_release(&fake).await;
        assert_eq!(
            accepted(&fake),
            vec![
                InputEvent::Button(Button::Left, true),
                InputEvent::Button(Button::Left, false)
            ]
        );
    }

    #[tokio::test]
    async fn partial_shortcut_failure_releases_the_accepted_modifier() {
        let (executor, fake) = executor(4, HOLD_LEASE);
        *fake.fail.lock().unwrap() = Some(1);
        let result = executor
            .execute(
                "a",
                InputCommand::KeyPress {
                    key: "ctrl+c".into(),
                },
                allow(),
            )
            .await;
        assert!(matches!(result, Err(InputError::Execution(_))));
        assert_eq!(
            accepted(&fake),
            vec![
                InputEvent::Key(InputKey::Virtual(0x11), true),
                InputEvent::Key(InputKey::Virtual(0x11), false)
            ]
        );
    }

    #[tokio::test]
    async fn rejected_input_never_reports_success_or_releases_unpressed_state() {
        let (executor, fake) = executor(4, HOLD_LEASE);
        *fake.fail.lock().unwrap() = Some(0);
        let result = executor.execute("a", down(), allow()).await;
        assert!(matches!(result, Err(InputError::Execution(_))));
        assert!(accepted(&fake).is_empty());
        assert_eq!(fake.batches.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn preexisting_local_input_is_not_borrowed_or_released() {
        let (executor, fake) = executor(4, HOLD_LEASE);
        fake.local_busy.store(true, Ordering::Release);
        assert!(matches!(
            executor
                .execute(
                    "a",
                    InputCommand::KeyPress {
                        key: "ctrl+c".into()
                    },
                    allow()
                )
                .await,
            Err(InputError::Busy)
        ));
        assert!(matches!(
            executor.execute("a", down(), allow()).await,
            Err(InputError::Busy)
        ));
        assert!(fake.batches.lock().unwrap().is_empty());
        fake.local_busy.store(false, Ordering::Release);
        executor.execute("a", movement(), allow()).await.unwrap();
    }

    #[tokio::test]
    async fn unicode_text_is_complete_and_has_no_per_character_sleep() {
        let (executor, fake) = executor(4, HOLD_LEASE);
        executor
            .execute(
                "a",
                InputCommand::TextInput {
                    text: "A😀".into()
                },
                allow(),
            )
            .await
            .unwrap();
        assert_eq!(
            accepted(&fake),
            vec![
                InputEvent::Key(InputKey::Unicode(0x41), true),
                InputEvent::Key(InputKey::Unicode(0x41), false),
                InputEvent::Key(InputKey::Unicode(0xD83D), true),
                InputEvent::Key(InputKey::Unicode(0xD83D), false),
                InputEvent::Key(InputKey::Unicode(0xDE00), true),
                InputEvent::Key(InputKey::Unicode(0xDE00), false)
            ]
        );
    }
}
