use std::cell::Cell;
use std::rc::Rc;

use futures::task::{AtomicWaker, Waker};

#[derive(Clone)]
/// Simple counter with ability to notify task on reaching specific number
///
/// Counter could be cloned, total ncount is shared across all clones.
pub struct Counter(Rc<CounterInner>);

struct CounterInner {
    count: Cell<usize>,
    capacity: usize,
    task: AtomicWaker,
}

impl Counter {
    /// Create `Counter` instance and set max value.
    pub fn new(capacity: usize) -> Self {
        Counter(Rc::new(CounterInner {
            capacity,
            count: Cell::new(0),
            task: AtomicWaker::new(),
        }))
    }

<<<<<<< HEAD
    /// Get counter guard.
    pub fn get(&self) -> CounterGuard {
        CounterGuard::new(self.0.clone())
=======
    pub fn get(&self, wake: &Waker) -> CounterGuard {
        CounterGuard::new(self.0.clone(), wake)
>>>>>>> Whole base codebase of actix-utils converted
    }

    /// Check if counter is not at capacity. If counter at capacity
    /// it registers notification for current task.
    pub fn available(&self) -> bool {
        self.0.available()
    }

    /// Get total number of acquired counts
    pub fn total(&self) -> usize {
        self.0.count.get()
    }
}

pub struct CounterGuard(Rc<CounterInner>);

impl CounterGuard {
    fn new(inner: Rc<CounterInner>, wake: &Waker) -> Self {
        inner.inc(wake);
        CounterGuard(inner)
    }
}

impl Drop for CounterGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}

impl CounterInner {
<<<<<<< HEAD
    fn inc(&self) {
        self.count.set(self.count.get() + 1);
=======
    fn inc(&self, wake: &Waker) {
        let num = self.count.get() + 1;
        self.count.set(num);
        if num == self.capacity {
            self.task.register(wake);
        }
>>>>>>> Whole base codebase of actix-utils converted
    }

    fn dec(&self) {
        let num = self.count.get();
        self.count.set(num - 1);
        if num == self.capacity {
            self.task.wake();
        }
    }

    fn available(&self) -> bool {
        if self.count.get() < self.capacity {
            true
        } else {
            self.task.register();
            false
        }
    }
}
