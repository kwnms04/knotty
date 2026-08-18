//! Telling the consumer there is something to come for, and the debt that
//! stands while nobody is there to be told.
//!
//! One of these per session, whichever shape the session comes in. A detached
//! session settles its own on the thread that drove it; a PTY session's is
//! registered from the app's thread and settled from the I/O thread, which is
//! why it carries a lock. cf. `03-core.md` C5

use std::sync::{Mutex, MutexGuard};

/// What a session calls when it has something new to be taken.
///
/// `Send` because a PTY session makes the call from its own I/O thread, which
/// is not the thread that registered it.
pub type Wake = Box<dyn Fn() + Send>;

/// A wake that has fallen due, and whoever is there to take it.
///
/// Owed with nobody to tell stays owed rather than being dropped, so a
/// consumer that registers late is told about what it was not there for
/// instead of having to know to go looking. The mailbox is holding a snapshot
/// either way. cf. `03-core.md` C5
#[derive(Default)]
pub struct Debt {
    owed: Mutex<Owed>,
}

/// Whether one is owed, and what to call to pay it.
///
/// The two live under the one lock because paying is reading them together:
/// a wake cleared between the two reads would be called after the consumer
/// said it had gone.
#[derive(Default)]
struct Owed {
    wake: Option<Wake>,
    standing: bool,
}

impl Debt {
    /// Set what to call when one falls due, or clear it with `None`, paying
    /// anything already owed before returning.
    pub fn register(&self, wake: Option<Wake>) {
        let mut owed = self.lock();
        owed.wake = wake;
        pay(&mut owed);
    }

    /// Record that there is something to come for.
    pub fn owe(&self) {
        self.lock().standing = true;
    }

    /// Whether one is owed, paid or not.
    pub fn owes(&self) -> bool {
        self.lock().standing
    }

    /// Pay what is owed, if anything is and anyone is there to take it.
    pub fn settle(&self) {
        pay(&mut self.lock());
    }

    fn lock(&self) -> MutexGuard<'_, Owed> {
        self.owed.lock().expect("wake lock")
    }
}

/// Call the wake if one is registered and one is owed, and clear the debt.
///
/// The call is made under the lock, which is what the wake contract already
/// allows for: a callback may do nothing but flag its own thread, so it never
/// comes back this way.
fn pay(owed: &mut Owed) {
    let Some(wake) = &owed.wake else {
        return;
    };
    if !owed.standing {
        return;
    }

    owed.standing = false;
    wake();
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::Debt;

    /// A debt and the count of what it paid.
    fn counting() -> (Debt, Arc<AtomicU32>) {
        let paid = Arc::new(AtomicU32::new(0));
        let debt = Debt::default();
        debt.register(Some(Box::new({
            let paid = Arc::clone(&paid);
            move || {
                paid.fetch_add(1, Ordering::Relaxed);
            }
        })));

        (debt, paid)
    }

    #[test]
    fn what_is_owed_is_paid_once() {
        let (debt, paid) = counting();

        debt.owe();
        debt.settle();
        debt.settle();

        assert_eq!(paid.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn settling_what_is_not_owed_pays_nothing() {
        let (debt, paid) = counting();

        debt.settle();

        assert_eq!(paid.load(Ordering::Relaxed), 0);
    }

    /// The whole reason a debt exists rather than a bare callback: a consumer
    /// that was not there for the publication still has to hear about it.
    #[test]
    fn a_debt_owed_to_nobody_is_paid_to_whoever_registers_next() {
        let debt = Debt::default();
        debt.owe();
        debt.settle();

        let paid = Arc::new(AtomicU32::new(0));
        debt.register(Some(Box::new({
            let paid = Arc::clone(&paid);
            move || {
                paid.fetch_add(1, Ordering::Relaxed);
            }
        })));

        assert_eq!(paid.load(Ordering::Relaxed), 1);
        assert!(!debt.owes(), "the debt was paid and still stands");
    }

    /// Clearing the callback is a consumer saying it is about to go, and what
    /// it would have been told has to survive that.
    #[test]
    fn clearing_the_wake_leaves_the_debt_where_it_is() {
        let (debt, paid) = counting();

        debt.register(None);
        debt.owe();
        debt.settle();

        assert_eq!(paid.load(Ordering::Relaxed), 0, "a cleared wake was called");
        assert!(debt.owes());
    }
}
