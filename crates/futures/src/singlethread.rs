use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::mem::ManuallyDrop;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use js_sys::Promise;
use wasm_bindgen::prelude::*;


struct Task {
    // This is an Option so that the Future can be immediately dropped when it's finished
    future: RefCell<Option<Pin<Box<dyn Future<Output = ()> + 'static>>>>,

    // This is used to ensure that the Task will only be queued once
    is_queued: Cell<bool>,
}

impl Task {
    #[inline]
    fn new<F>(future: F) -> Rc<Self>
    where
        F: Future<Output = ()> + 'static,
    {
        Rc::new(Self {
            future: RefCell::new(Some(Box::pin(future))),
            is_queued: Cell::new(false),
        })
    }

    // TODO guarantee that it will always be woken up on the same thread
    fn wake_by_ref(this: &Rc<Self>) {
        // This ensures that it's only queued once
        if this.is_queued.replace(true) {
            return;
        }

        EXECUTOR.with(|executor| {
            executor.push_task(this.clone());
        });
    }

    // This is to avoid a dependency on futures_util::task::ArcWake
    unsafe fn into_raw_waker(this: Rc<Self>) -> RawWaker {
        unsafe fn raw_clone(ptr: *const ()) -> RawWaker {
            let ptr = ManuallyDrop::new(Rc::from_raw(ptr as *const Task));
            Task::into_raw_waker((*ptr).clone())
        }

        unsafe fn raw_wake(ptr: *const ()) {
            let ptr = Rc::from_raw(ptr as *const Task);
            Task::wake_by_ref(&ptr);
        }

        unsafe fn raw_wake_by_ref(ptr: *const ()) {
            let ptr = ManuallyDrop::new(Rc::from_raw(ptr as *const Task));
            Task::wake_by_ref(&ptr);
        }

        unsafe fn raw_drop(ptr: *const ()) {
            drop(Rc::from_raw(ptr as *const Task));
        }

        const VTABLE: RawWakerVTable =
            RawWakerVTable::new(raw_clone, raw_wake, raw_wake_by_ref, raw_drop);

        RawWaker::new(Rc::into_raw(this) as *const (), &VTABLE)
    }
}


struct ExecutorState {
    // This is used to ensure that it's only scheduled once
    is_spinning: Cell<bool>,

    // This is a queue of Tasks which will be polled in order
    tasks: RefCell<VecDeque<Rc<Task>>>,
}

impl ExecutorState {
    fn run_tasks(&self) {
        loop {
            let mut lock = self.tasks.borrow_mut();

            match lock.pop_front() {
                Some(task) => {
                    let mut borrow = task.future.borrow_mut();

                    // This will only be None if the Future wakes up the Waker after returning Poll::Ready
                    if let Some(future) = borrow.as_mut() {
                        let poll = {
                            // Clear `is_queued` flag so that it will re-queue if poll calls waker.wake()
                            task.is_queued.set(false);

                            // This is necessary because the polled task might queue more tasks
                            drop(lock);

                            // TODO is there some way of saving these so they don't need to be recreated all the time ?
                            let waker = unsafe { Waker::from_raw(Task::into_raw_waker(task.clone())) };
                            let cx = &mut Context::from_waker(&waker);
                            future.as_mut().poll(cx)
                        };

                        if let Poll::Ready(_) = poll {
                            // Cleanup the Future immediately
                            *borrow = None;
                        }
                    }
                },
                None => {
                    // All of the Tasks have been polled, so it's now possible to schedule the next tick again
                    self.is_spinning.set(false);
                    break;
                },
            }
        }
    }
}


struct Executor {
    state: Rc<ExecutorState>,
    promise: Promise,
    closure: Closure<dyn FnMut(JsValue)>,
}

impl Executor {
    fn push_task(&self, task: Rc<Task>) {
        let mut lock = self.state.tasks.borrow_mut();

        lock.push_back(task);

        // If we already scheduled the next tick then do nothing
        if self.state.is_spinning.replace(true) {
            return;
        }

        // The Task will be polled on the next microtask event tick
        // TODO replace with https://developer.mozilla.org/en-US/docs/Web/API/WindowOrWorkerGlobalScope/queueMicrotask
        self.promise.then(&self.closure);
    }
}

thread_local! {
    static EXECUTOR: Executor = {
        let state = Rc::new(ExecutorState {
            is_spinning: Cell::new(false),
            tasks: RefCell::new(VecDeque::new()),
        });

        Executor {
            promise: Promise::resolve(&JsValue::null()),

            closure: {
                let state = state.clone();

                // This closure will only be called on the next microtask event tick
                Closure::wrap(Box::new(move |_| {
                    state.run_tasks();
                }))
            },

            state,
        }
    };
}


/// Runs a Rust `Future` on a local task queue.
///
/// The `future` provided must adhere to `'static` because it'll be scheduled
/// to run in the background and cannot contain any stack references.
///
/// # Panics
///
/// This function has the same panic behavior as `future_to_promise`.
pub fn spawn_local<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    Task::wake_by_ref(&Task::new(future));
}
