use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Applied,
    Refused(String),
    Retry(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TicketState {
    Pending,
    Applied,
    Refused(String),
    Failed(String),
}
#[derive(Clone)]
pub struct Ticket(Arc<Mutex<TicketState>>);
impl Ticket {
    pub fn state(&self) -> TicketState {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn same_ticket(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
struct Job<A> {
    action: A,
    key: String,
    ticket: Ticket,
}
struct Shared<A> {
    tickets: HashMap<String, Ticket>,
    sender: mpsc::Sender<Job<A>>,
}
pub struct Outbox<A> {
    shared: Arc<Mutex<Shared<A>>>,
}
impl<A> Clone for Outbox<A> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}
impl<A: Send + 'static> Outbox<A> {
    pub fn new(runner: impl Fn(&A, &str) -> Outcome + Send + Sync + 'static) -> Self {
        Self::with_max_attempts(runner, 5)
    }
    pub fn with_max_attempts(
        runner: impl Fn(&A, &str) -> Outcome + Send + Sync + 'static,
        max_attempts: usize,
    ) -> Self {
        let (sender, receiver) = mpsc::channel::<Job<A>>();
        thread::spawn(move || {
            for job in receiver {
                let max = max_attempts.max(1);
                // A panicking runner must not strand this or subsequent tickets.
                let mut result = None;
                for attempt in 0..max {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        runner(&job.action, &job.key)
                    }));
                    match outcome {
                        Ok(Outcome::Applied) => {
                            result = Some(TicketState::Applied);
                            break;
                        }
                        Ok(Outcome::Refused(reason)) => {
                            result = Some(TicketState::Refused(reason));
                            break;
                        }
                        Ok(Outcome::Retry(reason)) if attempt + 1 == max => {
                            result = Some(TicketState::Failed(reason));
                            break;
                        }
                        Ok(Outcome::Retry(_)) => thread::sleep(Duration::from_millis(
                            10u64.saturating_mul(1u64 << attempt.min(6)),
                        )),
                        Err(_) => {
                            result = Some(TicketState::Failed("runner panicked".into()));
                            break;
                        }
                    }
                }
                let state = result.expect("at least one attempt");
                *job.ticket.0.lock().unwrap_or_else(|e| e.into_inner()) = state;
            }
        });
        Self {
            shared: Arc::new(Mutex::new(Shared {
                tickets: HashMap::new(),
                sender,
            })),
        }
    }
    pub fn submit(&self, action: A, key: impl Into<String>) -> Ticket {
        let key = key.into();
        let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ticket) = shared.tickets.get(&key) {
            if matches!(ticket.state(), TicketState::Pending | TicketState::Applied) {
                return ticket.clone();
            }
        }
        let ticket = Ticket(Arc::new(Mutex::new(TicketState::Pending)));
        shared.tickets.insert(key.clone(), ticket.clone());
        if shared
            .sender
            .send(Job {
                action,
                key,
                ticket: ticket.clone(),
            })
            .is_err()
        {
            *ticket.0.lock().unwrap_or_else(|e| e.into_inner()) =
                TicketState::Failed("worker stopped".into());
        }
        ticket
    }
    pub fn tickets(&self) -> Vec<Ticket> {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .tickets
            .values()
            .cloned()
            .collect()
    }
}
