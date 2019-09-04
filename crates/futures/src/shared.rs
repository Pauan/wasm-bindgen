use js_sys::Promise;
use std::cell::RefCell;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use wasm_bindgen::prelude::*;


struct Inner {
    result: Option<Result<JsValue, JsValue>>,
    task: Option<Waker>,
    callbacks: Option<(Closure<dyn FnMut(JsValue)>, Closure<dyn FnMut(JsValue)>)>,
}

/// A Rust `Future` backed by a JavaScript `Promise`.
///
/// This type is constructed with a JavaScript `Promise` object and translates
/// it to a Rust `Future`. This type implements the `Future` trait from the
/// `futures` crate and will either succeed or fail depending on what happens
/// with the JavaScript `Promise`.
///
/// Currently this type is constructed with `JsFuture::from`.
pub struct JsFuture {
    inner: Rc<RefCell<Inner>>,
}

impl fmt::Debug for JsFuture {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "JsFuture {{ ... }}")
    }
}

impl From<Promise> for JsFuture {
    fn from(js: Promise) -> JsFuture {
        // Use the `then` method to schedule two callbacks, one for the
        // resolved value and one for the rejected value. We're currently
        // assuming that JS engines will unconditionally invoke precisely one of
        // these callbacks, no matter what.
        //
        // Ideally we'd have a way to cancel the callbacks getting invoked and
        // free up state ourselves when this `JsFuture` is dropped. We don't
        // have that, though, and one of the callbacks is likely always going to
        // be invoked.
        //
        // As a result we need to make sure that no matter when the callbacks
        // are invoked they are valid to be called at any time, which means they
        // have to be self-contained. Through the `Closure::once` and some
        // `Rc`-trickery we can arrange for both instances of `Closure`, and the
        // `Rc`, to all be destroyed once the first one is called.
        let state = Rc::new(RefCell::new(Inner {
            result: None,
            task: None,
            callbacks: None,
        }));

        fn finish(state: &RefCell<Inner>, val: Result<JsValue, JsValue>) {
            // First up drop our closures as they'll never be invoked again and
            // this is our chance to clean up their state. Next store the value
            // into the internal state, and then finally if any task was waiting
            // on the value wake it up and let them know it's there.
            let task = {
                let mut state = state.borrow_mut();
                debug_assert!(state.callbacks.is_some());
                debug_assert!(state.result.is_none());
                drop(state.callbacks.take());
                state.result = Some(val);
                state.task.take()
            };

            if let Some(task) = task {
                task.wake()
            }
        }

        let resolve = {
            let state = state.clone();
            Closure::once(move |val| finish(&state, Ok(val)))
        };

        let reject = {
            let state = state.clone();
            Closure::once(move |val| finish(&state, Err(val)))
        };

        js.then2(&resolve, &reject);

        state.borrow_mut().callbacks = Some((resolve, reject));

        JsFuture { inner: state }
    }
}

impl Future for JsFuture {
    type Output = Result<JsValue, JsValue>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let mut inner = self.inner.borrow_mut();

        // If our value has come in then we return it...
        if let Some(val) = inner.result.take() {
            return Poll::Ready(val);
        }

        // ... otherwise we arrange ourselves to get woken up once the value
        // does come in
        inner.task = Some(cx.waker().clone());
        Poll::Pending
    }
}


/// Converts a Rust `Future` into a JavaScript `Promise`.
///
/// This function will take any future in Rust and schedule it to be executed,
/// returning a JavaScript `Promise` which can then be passed back to JavaScript
/// to get plumbed into the rest of a system.
///
/// The `future` provided must adhere to `'static` because it'll be scheduled
/// to run in the background and cannot contain any stack references. The
/// returned `Promise` will be resolved or rejected when the future completes,
/// depending on whether it finishes with `Ok` or `Err`.
///
/// # Panics
///
/// Note that in wasm panics are currently translated to aborts, but "abort" in
/// this case means that a JavaScript exception is thrown. The wasm module is
/// still usable (likely erroneously) after Rust panics.
///
/// If the `future` provided panics then the returned `Promise` **will not
/// resolve**. Instead it will be a leaked promise. This is an unfortunate
/// limitation of wasm currently that's hoped to be fixed one day!
pub fn future_to_promise<F>(future: F) -> Promise
where
    F: Future<Output = Result<JsValue, JsValue>> + 'static,
{
    let mut future = Some(future);

    Promise::new(&mut |resolve, reject| {
        let future = future.take().unwrap_throw();

        // TODO change Promise::new to be FnOnce
        crate::spawn_local(async move {
            match future.await {
                Ok(val) => {
                    resolve.call1(&JsValue::undefined(), &val).unwrap_throw();
                }
                Err(val) => {
                    reject.call1(&JsValue::undefined(), &val).unwrap_throw();
                }
            }
        });
    })
}
