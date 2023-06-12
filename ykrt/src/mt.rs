//! The main end-user interface to the meta-tracing system.

#[cfg(feature = "yk_testing")]
use std::env;
use std::{
    cell::RefCell,
    cmp,
    collections::VecDeque,
    error::Error,
    ffi::c_void,
    marker::PhantomData,
    sync::{
        atomic::{AtomicU16, AtomicU32, AtomicUsize, Ordering},
        Arc,
    },
    thread,
};

use parking_lot::{Condvar, Mutex, MutexGuard};
use parking_lot_core::SpinWait;
#[cfg(feature = "yk_jitstate_debug")]
use std::sync::LazyLock;

#[cfg(feature = "yk_jitstate_debug")]
use crate::print_jit_state;
use crate::{
    location::{HotLocation, HotLocationKind, Location},
    trace::{default_tracer_for_platform, CompiledTrace, ThreadTracer, Tracer, UnmappedTrace},
};
use yktracec::promote;

// The HotThreshold must be less than a machine word wide for [`Location::Location`] to do its
// pointer tagging thing. We therefore choose a type which makes this statically clear to
// users rather than having them try to use (say) u64::max() on a 64 bit machine and get a run-time
// error.
#[cfg(target_pointer_width = "64")]
pub type HotThreshold = u32;
#[cfg(target_pointer_width = "64")]
type AtomicHotThreshold = AtomicU32;

pub type TraceFailureThreshold = u16;
pub type AtomicTraceFailureThreshold = AtomicU16;

const DEFAULT_HOT_THRESHOLD: HotThreshold = 50;
const DEFAULT_TRACE_FAILURE_THRESHOLD: TraceFailureThreshold = 5;

thread_local! {static THREAD_MTTHREAD: MTThread = MTThread::new();}

#[cfg(feature = "yk_testing")]
static SERIALISE_COMPILATION: LazyLock<bool> = LazyLock::new(|| {
    &env::var("YKD_SERIALISE_COMPILATION").unwrap_or_else(|_| "0".to_owned()) == "1"
});

/// A meta-tracer. Note that this is conceptually a "front-end" to the actual meta-tracer akin to
/// an `Rc`: this struct can be freely `clone()`d without duplicating the underlying meta-tracer.
pub struct MT {
    hot_threshold: AtomicHotThreshold,
    trace_failure_threshold: AtomicTraceFailureThreshold,
    /// The ordered queue of compilation worker functions.
    job_queue: Arc<(Condvar, Mutex<VecDeque<Box<dyn FnOnce() + Send>>>)>,
    /// The hard cap on the number of worker threads.
    max_worker_threads: AtomicUsize,
    /// How many worker threads are currently running. Note that this may temporarily be `>`
    /// [`max_worker_threads`].
    active_worker_threads: AtomicUsize,
    tracer: Arc<dyn Tracer>,
}

impl MT {
    // Create a new meta-tracer instance. Arbitrarily many of these can be created, though there
    // are no guarantees as to whether they will share resources effectively or fairly.
    pub fn new() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            hot_threshold: AtomicHotThreshold::new(DEFAULT_HOT_THRESHOLD),
            trace_failure_threshold: AtomicTraceFailureThreshold::new(
                DEFAULT_TRACE_FAILURE_THRESHOLD,
            ),
            job_queue: Arc::new((Condvar::new(), Mutex::new(VecDeque::new()))),
            max_worker_threads: AtomicUsize::new(cmp::max(1, num_cpus::get() - 1)),
            active_worker_threads: AtomicUsize::new(0),
            tracer: default_tracer_for_platform()?,
        })
    }

    /// Return this `MT` instance's current hot threshold. Notice that this value can be changed by
    /// other threads and is thus potentially stale as soon as it is read.
    pub fn hot_threshold(&self) -> HotThreshold {
        self.hot_threshold.load(Ordering::Relaxed)
    }

    /// Set the threshold at which `Location`'s are considered hot.
    pub fn set_hot_threshold(&self, hot_threshold: HotThreshold) {
        self.hot_threshold.store(hot_threshold, Ordering::Relaxed);
    }

    /// Return this `MT` instance's current trace failure threshold. Notice that this value can be
    /// changed by other threads and is thus potentially stale as soon as it is read.
    pub fn trace_failure_threshold(&self) -> TraceFailureThreshold {
        self.trace_failure_threshold.load(Ordering::Relaxed)
    }

    /// Set the threshold at which a `Location` from which tracing has failed multiple times is
    /// marked as "do not try tracing again".
    pub fn set_trace_failure_threshold(&self, trace_failure_threshold: TraceFailureThreshold) {
        if trace_failure_threshold < 1 {
            panic!("Trace failure threshold must be >= 1.");
        }
        self.trace_failure_threshold
            .store(trace_failure_threshold, Ordering::Relaxed);
    }

    /// Return this meta-tracer's maximum number of worker threads. Notice that this value can be
    /// changed by other threads and is thus potentially stale as soon as it is read.
    pub fn max_worker_threads(&self) -> usize {
        self.max_worker_threads.load(Ordering::Relaxed)
    }

    /// Queue `job` to be run on a worker thread.
    fn queue_job(&self, job: Box<dyn FnOnce() + Send>) {
        // We have a very simple model of worker threads. Each time a job is queued, we spin up a
        // new worker thread iff we aren't already running the maximum number of worker threads.
        // Once started, a worker thread never dies, waiting endlessly for work.

        let (cv, mtx) = &*self.job_queue;
        mtx.lock().push_back(job);
        cv.notify_one();

        let max_jobs = self.max_worker_threads.load(Ordering::Relaxed);
        if self.active_worker_threads.load(Ordering::Relaxed) < max_jobs {
            // At the point of the `load` on the previous line, we weren't running the maximum
            // number of worker threads. There is now a possible race condition where multiple
            // threads calling `queue_job` could try creating multiple worker threads and push us
            // over the maximum worker thread limit.
            if self.active_worker_threads.fetch_add(1, Ordering::Relaxed) > max_jobs {
                // Another thread(s) is also spinning up another worker thread and they won the
                // race.
                self.active_worker_threads.fetch_sub(1, Ordering::Relaxed);
                return;
            }

            let jq = Arc::clone(&self.job_queue);
            thread::spawn(move || {
                let (cv, mtx) = &*jq;
                let mut lock = mtx.lock();
                loop {
                    match lock.pop_front() {
                        Some(x) => MutexGuard::unlocked(&mut lock, x),
                        None => cv.wait(&mut lock),
                    }
                }
            });
        }
    }

    pub fn control_point(
        &self,
        loc: &Location,
        ctrlp_vars: *mut c_void,
        frameaddr: *mut c_void,
    ) -> *const c_void {
        match self.transition_location(loc) {
            TransitionLocation::NoAction => (),
            TransitionLocation::Execute(ctr) => {
                // FIXME: If we want to free compiled traces, we'll need to refcount (or use
                // a GC) to know if anyone's executing that trace at the moment.
                #[cfg(feature = "yk_jitstate_debug")]
                print_jit_state("enter-jit-code");
                let ptr = ctr.exec(ctrlp_vars, frameaddr);
                #[cfg(feature = "yk_jitstate_debug")]
                print_jit_state("exit-jit-code");
                return ptr;
            }
            TransitionLocation::StartTracing => {
                #[cfg(feature = "yk_jitstate_debug")]
                print_jit_state("start-tracing");
                let tracer = Arc::clone(&self.tracer);
                match Arc::clone(&tracer).start_collector() {
                    Ok(tt) => THREAD_MTTHREAD.with(|mtt| {
                        promote::thread_record_enable(true);
                        *mtt.thread_tracer.borrow_mut() = Some((tracer, tt));
                    }),
                    Err(e) => todo!("{e:?}"),
                }
            }
            TransitionLocation::StopTracing(x) => {
                promote::thread_record_enable(false);
                // Assuming no bugs elsewhere, the `unwrap` cannot fail, because `StartTracing`
                // will have put a `Some` in the `Rc`.
                let (trcr, thrdtrcr) =
                    THREAD_MTTHREAD.with(|mtt| mtt.thread_tracer.take().unwrap());
                match thrdtrcr.stop_collector() {
                    Ok(utrace) => {
                        #[cfg(feature = "yk_jitstate_debug")]
                        print_jit_state("stop-tracing");
                        self.queue_compile_job(utrace, x, trcr);
                    }
                    Err(_) => todo!(),
                }
            }
        }
        std::ptr::null()
    }

    /// Perform the next step to `loc` in the `Location` state-machine. If `loc` moves to the
    /// Compiled state, return a pointer to a [CompiledTrace] object.
    fn transition_location(&self, loc: &Location) -> TransitionLocation {
        let am_tracing = THREAD_MTTHREAD.with(|mtt| mtt.tracing.borrow().is_some());
        match loc.hot_location() {
            Some(hl) => {
                // If this thread is tracing something, we *must* grab the [HotLocation] lock,
                // because we need to know for sure if `loc` is the point at which we should stop
                // tracing. If this thread is not tracing anything, however, it's not worth
                // contending too much with other threads: we try moderately hard to grab the lock,
                // but we don't want to park this thread.
                let mut lk = if !am_tracing {
                    // This thread isn't tracing anything, so we try for a little while to grab the
                    // lock, before giving up and falling back to the interpreter. In general, we
                    // expect that we'll grab the lock rather quickly. However, there is one nasty
                    // use-case, which is when an army of threads all start executing the same
                    // piece of tiny code and end up thrashing away at a single Location,
                    // particularly when it's in a non-Compiled state: we can end up contending
                    // horribly for a single lock, and not making much progress. In that case, it's
                    // probably better to let some threads fall back to the interpreter for another
                    // iteration, and hopefully allow them to get sufficiently out-of-sync that
                    // they no longer contend on this one lock as much.
                    let mut sw = SpinWait::new();
                    loop {
                        if let Some(lk) = hl.try_lock() {
                            break lk;
                        }
                        if !sw.spin() {
                            return TransitionLocation::NoAction;
                        }
                    }
                } else {
                    // This thread is tracing something, so we must grab the lock.
                    hl.lock()
                };

                match lk.kind {
                    HotLocationKind::Compiled(ref ctr) => {
                        if am_tracing {
                            // This thread is tracing something, so bail out as quickly as possible
                            TransitionLocation::NoAction
                        } else {
                            TransitionLocation::Execute(Arc::clone(ctr))
                        }
                    }
                    HotLocationKind::Compiling(ref arcmtx) => {
                        if am_tracing {
                            // This thread is tracing something, so bail out as quickly as possible
                            TransitionLocation::NoAction
                        } else {
                            match arcmtx.try_lock().map(|mut x| x.take()) {
                                None | Some(None) => {
                                    // `None` means we failed to grab the lock; `Some(None)` means we
                                    // grabbed the lock but compilation has not yet completed.
                                    TransitionLocation::NoAction
                                }
                                Some(Some(ctr)) => {
                                    lk.kind = HotLocationKind::Compiled(Arc::clone(&ctr));
                                    TransitionLocation::Execute(ctr)
                                }
                            }
                        }
                    }
                    HotLocationKind::Tracing(_) => {
                        THREAD_MTTHREAD.with(|mtt| {
                            let hl = loc.hot_location_arc_clone().unwrap();
                            let mut thread_hl_out = mtt.tracing.borrow_mut();
                            if let Some(ref thread_hl_in) = *thread_hl_out {
                                // This thread is tracing something...
                                if !Arc::ptr_eq(thread_hl_in, &hl) {
                                    // ...but not this Location.
                                    TransitionLocation::NoAction
                                } else {
                                    // ...and it's this location: we have therefore finished tracing the loop.
                                    *thread_hl_out = None;
                                    let mtx = Arc::new(Mutex::new(None));
                                    lk.kind = HotLocationKind::Compiling(Arc::clone(&mtx));
                                    TransitionLocation::StopTracing(mtx)
                                }
                            } else {
                                // This thread isn't tracing anything. Note that because we called
                                // `hot_location_arc_clone` above, the strong count of an `Arc`
                                // that's no longer being used by that thread will be 2.
                                if Arc::strong_count(&hl) == 2 {
                                    // Another thread was tracing this location but it's terminated.
                                    if lk.trace_failure < self.trace_failure_threshold() {
                                        // Let's try tracing the location again in this thread.
                                        lk.trace_failure += 1;
                                        lk.kind = HotLocationKind::Tracing(0);
                                        *thread_hl_out = Some(Arc::clone(&hl));
                                        TransitionLocation::StartTracing
                                    } else {
                                        // This location has failed too many times: don't try tracing it
                                        // again.
                                        lk.kind = HotLocationKind::DontTrace;
                                        TransitionLocation::NoAction
                                    }
                                } else {
                                    // Another thread is tracing this location.
                                    TransitionLocation::NoAction
                                }
                            }
                        })
                    }
                    HotLocationKind::DontTrace => TransitionLocation::NoAction,
                }
            }
            None => {
                if am_tracing {
                    // This thread is tracing something, so bail out as quickly as possible
                    return TransitionLocation::NoAction;
                }
                match loc.count() {
                    Some(x) => {
                        if x < self.hot_threshold() {
                            loc.count_set(x, x + 1);
                            TransitionLocation::NoAction
                        } else {
                            let hl = HotLocation {
                                kind: HotLocationKind::Tracing(0),
                                trace_failure: 0,
                            };
                            if let Some(hl) = loc.count_to_hot_location(x, hl) {
                                THREAD_MTTHREAD.with(|mtt| {
                                    debug_assert!(mtt.tracing.borrow().is_none());
                                    *mtt.tracing.borrow_mut() = Some(hl);
                                });
                                TransitionLocation::StartTracing
                            } else {
                                // We raced with another thread which has started tracing this
                                // location. We leave it to do the tracing.
                                TransitionLocation::NoAction
                            }
                        }
                    }
                    None => {
                        // `loc` is being updated by another thread and we've caught it in the
                        // middle of that. We could spin but we might as well let the other thread
                        // do its thing and go around the interpreter again.
                        TransitionLocation::NoAction
                    }
                }
            }
        }
    }

    /// Add a compilation job for `sir` to the global work queue.
    fn queue_compile_job(
        &self,
        utrace: Box<dyn UnmappedTrace>,
        mtx: Arc<Mutex<Option<Arc<CompiledTrace>>>>,
        tracer: Arc<dyn Tracer>,
    ) {
        let do_compile = move || {
            // FIXME: if mapping or tracing fails we don't want to abort, but in order to do that,
            // we'll need to move the location into something other than the Compiling state.
            let irtrace = match utrace.map(tracer) {
                Ok(x) => x,
                Err(e) => todo!("{e:?}"),
            };
            match irtrace.compile() {
                Ok((codeptr, di_tmpfile)) => {
                    let ct = Arc::new(CompiledTrace::new(codeptr, di_tmpfile));
                    // FIXME: although we've now put the compiled trace into the `HotLocation`,
                    // there's no guarantee that the `Location` for which we're compiling will ever
                    // be executed again. In such a case, the memory has, in essence, leaked.
                    mtx.lock().replace(ct);
                }
                Err(_e) => {
                    // FIXME: Properly handle failed trace compilation, e.g. depending on the
                    // reason for the failure we might want to block this location from being
                    // traced again or only temporarily put it on hold and try again later.
                    // See: https://github.com/ykjit/yk/issues/612
                    // FIXME: Improve jit-state message.
                    // See: https://github.com/ykjit/yk/issues/611
                    //
                    #[cfg(feature = "yk_jitstate_debug")]
                    print_jit_state("trace-compilation-aborted");
                }
            };
        };

        #[cfg(feature = "yk_testing")]
        if *SERIALISE_COMPILATION {
            do_compile();
            return;
        }

        self.queue_job(Box::new(do_compile));
    }
}

/// Meta-tracer per-thread state. Note that this struct is neither `Send` nor `Sync`: it can only
/// be accessed from within a single thread.
pub struct MTThread {
    /// Is this thread currently tracing something? If so, this will be a `Some<...>`. This allows
    /// another thread to tell whether the thread that started tracing a [Location] is still alive
    /// or not by inspecting its strong count (if the strong count is equal to 1 then the thread
    /// died while tracing). Note that this relies on thread local storage dropping the [MTThread]
    /// instance and (by implication) dropping the [Arc] and decrementing its strong count.
    /// Unfortunately, there is no guarantee that thread local storage will be dropped when a
    /// thread dies (and there is also significant platform variation in regard to dropping thread
    /// locals), so this mechanism can't be fully relied upon: however, we can't monitor thread
    /// death in any other reasonable way, so this will have to do.
    tracing: RefCell<Option<Arc<Mutex<HotLocation>>>>,
    /// When tracing is active, this will be `RefCell<Some(...)>`; when tracing is inactive
    /// `RefCell<None>`. We need to keep track of the [Tracer] used to start the [ThreadTracer], as
    /// trace mapping requires a reference to the [Tracer].
    thread_tracer: RefCell<Option<(Arc<dyn Tracer>, Box<dyn ThreadTracer>)>>,
    // Raw pointers are neither send nor sync.
    _dont_send_or_sync_me: PhantomData<*mut ()>,
}

impl MTThread {
    fn new() -> Self {
        MTThread {
            tracing: RefCell::new(None),
            thread_tracer: RefCell::new(None),
            _dont_send_or_sync_me: PhantomData,
        }
    }
}

/// What action should a caller of `MT::transition_location` take?
#[derive(Debug)]
enum TransitionLocation {
    NoAction,
    Execute(Arc<CompiledTrace>),
    StartTracing,
    StopTracing(Arc<Mutex<Option<Arc<CompiledTrace>>>>),
}

#[cfg(test)]
impl PartialEq for TransitionLocation {
    fn eq(&self, other: &Self) -> bool {
        // We only implement enough of the equality function for the tests we have.
        match (self, other) {
            (TransitionLocation::NoAction, TransitionLocation::NoAction) => true,
            (TransitionLocation::Execute(p1), TransitionLocation::Execute(p2)) => {
                std::ptr::eq(p1, p2)
            }
            (TransitionLocation::StartTracing, TransitionLocation::StartTracing) => true,
            (x, y) => todo!("{:?} {:?}", x, y),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate test;
    use super::*;
    use crate::location::HotLocationKind;
    use std::{convert::TryFrom, hint::black_box, sync::atomic::AtomicU64, thread};
    use test::bench::Bencher;

    #[test]
    fn basic_transitions() {
        let hot_thrsh = 5;
        let mt = MT::new().unwrap();
        mt.set_hot_threshold(hot_thrsh);
        let loc = Location::new();
        for i in 0..mt.hot_threshold() {
            assert_eq!(mt.transition_location(&loc), TransitionLocation::NoAction);
            assert_eq!(loc.count(), Some(i + 1));
        }
        assert_eq!(
            mt.transition_location(&loc),
            TransitionLocation::StartTracing
        );
        assert!(matches!(
            loc.hot_location().unwrap().lock().kind,
            HotLocationKind::Tracing(_)
        ));
        match mt.transition_location(&loc) {
            TransitionLocation::StopTracing(mtx) => {
                assert!(matches!(
                    loc.hot_location().unwrap().lock().kind,
                    HotLocationKind::Compiling(_)
                ));
                *mtx.lock() = Some(Arc::new(unsafe { CompiledTrace::new_null() }));
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            mt.transition_location(&loc),
            TransitionLocation::Execute(_)
        ));
    }

    #[test]
    fn threaded_threshold() {
        // Aim for a situation where there's a lot of contention.
        let num_threads = u32::try_from(num_cpus::get() * 4).unwrap();
        let hot_thrsh = num_threads.saturating_mul(100000);
        let mt = Arc::new(MT::new().unwrap());
        mt.set_hot_threshold(hot_thrsh);
        let loc = Arc::new(Location::new());

        let mut thrs = vec![];
        for _ in 0..num_threads {
            let mt = Arc::clone(&mt);
            let loc = Arc::clone(&loc);
            let t = thread::spawn(move || {
                // The "*4" is the number of times we call `transition_location` in the loop: we
                // need to make sure that this loop cannot tip the Location over the threshold,
                // otherwise tracing will start, and the assertions will fail.
                for _ in 0..hot_thrsh / (num_threads * 4) {
                    assert_eq!(mt.transition_location(&loc), TransitionLocation::NoAction);
                    let c1 = loc.count();
                    assert!(c1.is_some());
                    assert_eq!(mt.transition_location(&loc), TransitionLocation::NoAction);
                    let c2 = loc.count();
                    assert!(c2.is_some());
                    assert_eq!(mt.transition_location(&loc), TransitionLocation::NoAction);
                    let c3 = loc.count();
                    assert!(c3.is_some());
                    assert_eq!(mt.transition_location(&loc), TransitionLocation::NoAction);
                    let c4 = loc.count();
                    assert!(c4.is_some());
                    assert!(c4.unwrap() >= c3.unwrap());
                    assert!(c3.unwrap() >= c2.unwrap());
                    assert!(c2.unwrap() >= c1.unwrap());
                }
            });
            thrs.push(t);
        }
        for t in thrs {
            t.join().unwrap();
        }
        // Thread contention and the use of `compare_exchange_weak` means that there is absolutely
        // no guarantee about what the location's count will be at this point other than it must be
        // at or below the threshold: it could even be (although it's rather unlikely) 0!
        assert!(loc.count().is_some());
        loop {
            match mt.transition_location(&loc) {
                TransitionLocation::NoAction => (),
                TransitionLocation::StartTracing => break,
                _ => unreachable!(),
            }
        }
        assert!(matches!(
            mt.transition_location(&loc),
            TransitionLocation::StopTracing(_)
        ));
        // At this point, we have nothing to meaningfully test over the `basic_transitions` test.
    }

    #[test]
    fn locations_dont_get_stuck_tracing() {
        // If tracing a location fails too many times (e.g. because the thread terminates before
        // tracing is complete), the location must be marked as DontTrace.

        const THRESHOLD: HotThreshold = 5;
        let mt = Arc::new(MT::new().unwrap());
        mt.set_hot_threshold(THRESHOLD);
        let loc = Arc::new(Location::new());

        // Get the location to the point of being hot.
        for _ in 0..THRESHOLD {
            assert_eq!(mt.transition_location(&loc), TransitionLocation::NoAction);
        }

        // Start tracing in a thread and purposefully let the thread terminate before tracing is
        // complete.
        for i in 0..mt.trace_failure_threshold() + 1 {
            {
                let mt = Arc::clone(&mt);
                let loc = Arc::clone(&loc);
                thread::spawn(move || {
                    assert!(matches!(
                        mt.transition_location(&loc),
                        TransitionLocation::StartTracing
                    ));
                })
                .join()
                .unwrap();
            }
            assert!(matches!(
                loc.hot_location().unwrap().lock().kind,
                HotLocationKind::Tracing(_)
            ));
            assert_eq!(loc.hot_location().unwrap().lock().trace_failure, i);
        }

        assert!(matches!(
            loc.hot_location().unwrap().lock().kind,
            HotLocationKind::Tracing(_)
        ));
        assert_eq!(mt.transition_location(&loc), TransitionLocation::NoAction);
        assert!(matches!(
            loc.hot_location().unwrap().lock().kind,
            HotLocationKind::DontTrace
        ));
    }

    #[test]
    fn locations_can_fail_tracing_before_succeeding() {
        // Test that a location can fail tracing multiple times before being successfully traced.

        const THRESHOLD: HotThreshold = 5;
        let mt = Arc::new(MT::new().unwrap());
        mt.set_hot_threshold(THRESHOLD);
        let loc = Arc::new(Location::new());

        // Get the location to the point of being hot.
        for _ in 0..THRESHOLD {
            assert_eq!(mt.transition_location(&loc), TransitionLocation::NoAction);
        }

        // Start tracing in a thread and purposefully let the thread terminate before tracing is
        // complete.
        for i in 0..mt.trace_failure_threshold() {
            {
                let mt = Arc::clone(&mt);
                let loc = Arc::clone(&loc);
                thread::spawn(move || {
                    assert!(matches!(
                        mt.transition_location(&loc),
                        TransitionLocation::StartTracing
                    ));
                })
                .join()
                .unwrap();
            }
            assert!(matches!(
                loc.hot_location().unwrap().lock().kind,
                HotLocationKind::Tracing(_)
            ));
            assert_eq!(loc.hot_location().unwrap().lock().trace_failure, i);
        }

        assert!(matches!(
            loc.hot_location().unwrap().lock().kind,
            HotLocationKind::Tracing(_)
        ));
        // Start tracing again...
        assert!(matches!(
            mt.transition_location(&loc),
            TransitionLocation::StartTracing
        ));
        assert!(matches!(
            loc.hot_location().unwrap().lock().kind,
            HotLocationKind::Tracing(_)
        ));
        // ...and this time let tracing succeed.
        assert!(matches!(
            mt.transition_location(&loc),
            TransitionLocation::StopTracing(_)
        ));
        // If tracing succeeded, we'll now be in the Compiling state.
        assert!(matches!(
            loc.hot_location().unwrap().lock().kind,
            HotLocationKind::Compiling(_)
        ));
    }

    #[test]
    fn dont_trace_two_locations_simultaneously_in_one_thread() {
        // A thread can only trace one Location at a time: if, having started tracing, it
        // encounters another Location which has reached its hot threshold, it just ignores it.
        // Once the first location is compiled, the second location can then be compiled.

        const THRESHOLD: HotThreshold = 5;
        let mt = MT::new().unwrap();
        mt.set_hot_threshold(THRESHOLD);
        let loc1 = Location::new();
        let loc2 = Location::new();

        for _ in 0..THRESHOLD {
            assert_eq!(mt.transition_location(&loc1), TransitionLocation::NoAction);
            assert_eq!(mt.transition_location(&loc2), TransitionLocation::NoAction);
        }
        assert!(matches!(
            mt.transition_location(&loc1),
            TransitionLocation::StartTracing
        ));
        assert_eq!(mt.transition_location(&loc2), TransitionLocation::NoAction);
        assert!(matches!(
            loc1.hot_location().unwrap().lock().kind,
            HotLocationKind::Tracing(_)
        ));
        assert_eq!(loc2.count(), Some(THRESHOLD));
        assert!(matches!(
            mt.transition_location(&loc1),
            TransitionLocation::StopTracing(_)
        ));
        assert!(matches!(
            loc1.hot_location().unwrap().lock().kind,
            HotLocationKind::Compiling(_)
        ));
        assert!(matches!(
            mt.transition_location(&loc2),
            TransitionLocation::StartTracing
        ));
        assert!(matches!(
            mt.transition_location(&loc2),
            TransitionLocation::StopTracing(_)
        ));
    }

    #[test]
    fn only_one_thread_starts_tracing() {
        // If multiple threads hammer away at a location, only one of them can win the race to
        // trace it (and then compile it etc.).

        // We need to set a high enough threshold that the threads are likely to meaningfully
        // interleave when interacting with the location.
        const THRESHOLD: HotThreshold = 100000;
        let mt = Arc::new(MT::new().unwrap());
        mt.set_hot_threshold(THRESHOLD);
        let loc = Arc::new(Location::new());

        let mut thrs = Vec::new();
        let num_starts = Arc::new(AtomicU64::new(0));
        for _ in 0..num_cpus::get() {
            let loc = Arc::clone(&loc);
            let mt = Arc::clone(&mt);
            let num_starts = Arc::clone(&num_starts);
            thrs.push(thread::spawn(move || {
                for _ in 0..THRESHOLD {
                    match mt.transition_location(&loc) {
                        TransitionLocation::NoAction => (),
                        TransitionLocation::Execute(_) => (),
                        TransitionLocation::StartTracing => {
                            num_starts.fetch_add(1, Ordering::Relaxed);
                            assert!(matches!(
                                loc.hot_location().unwrap().lock().kind,
                                HotLocationKind::Tracing(_)
                            ));

                            match mt.transition_location(&loc) {
                                TransitionLocation::StopTracing(mtx) => {
                                    assert!(matches!(
                                        loc.hot_location().unwrap().lock().kind,
                                        HotLocationKind::Compiling(_)
                                    ));
                                    assert_eq!(
                                        mt.transition_location(&loc),
                                        TransitionLocation::NoAction
                                    );
                                    assert!(matches!(
                                        loc.hot_location().unwrap().lock().kind,
                                        HotLocationKind::Compiling(_)
                                    ));
                                    *mtx.lock() =
                                        Some(Arc::new(unsafe { CompiledTrace::new_null() }));
                                }
                                x => unreachable!("Reached incorrect state {:?}", x),
                            }
                            loop {
                                if let TransitionLocation::Execute(_) = mt.transition_location(&loc)
                                {
                                    break;
                                }
                            }
                            break;
                        }
                        TransitionLocation::StopTracing(_) => unreachable!(),
                    }
                }
            }));
        }

        for t in thrs {
            t.join().unwrap();
        }

        assert_eq!(num_starts.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn two_tracing_threads_must_not_stop_each_others_tracing_location() {
        // A tracing thread can only stop tracing when it encounters the specific Location that
        // caused it to start tracing. If it encounters another Location that also happens to be
        // tracing, it must ignore it.

        const THRESHOLD: HotThreshold = 5;
        let mt = Arc::new(MT::new().unwrap());
        mt.set_hot_threshold(THRESHOLD);
        let loc1 = Arc::new(Location::new());
        let loc2 = Location::new();

        for _ in 0..THRESHOLD {
            assert_eq!(mt.transition_location(&loc1), TransitionLocation::NoAction);
            assert_eq!(mt.transition_location(&loc2), TransitionLocation::NoAction);
        }

        {
            let mt = Arc::clone(&mt);
            let loc1 = Arc::clone(&loc1);
            thread::spawn(move || {
                assert!(matches!(
                    mt.transition_location(&loc1),
                    TransitionLocation::StartTracing
                ));
            })
            .join()
            .unwrap();
        }

        assert!(matches!(
            mt.transition_location(&loc2),
            TransitionLocation::StartTracing
        ));
        assert_eq!(mt.transition_location(&loc1), TransitionLocation::NoAction);
        assert!(matches!(
            mt.transition_location(&loc2),
            TransitionLocation::StopTracing(_)
        ));
    }

    #[bench]
    fn bench_single_threaded_control_point(b: &mut Bencher) {
        let mt = MT::new().unwrap();
        let loc = Location::new();
        b.iter(|| {
            for _ in 0..100000 {
                black_box(mt.transition_location(&loc));
            }
        });
    }

    #[bench]
    fn bench_multi_threaded_control_point(b: &mut Bencher) {
        let mt = Arc::new(MT::new().unwrap());
        let loc = Arc::new(Location::new());
        b.iter(|| {
            let mut thrs = vec![];
            for _ in 0..4 {
                let loc = Arc::clone(&loc);
                let mt = Arc::clone(&mt);
                thrs.push(thread::spawn(move || {
                    for _ in 0..100 {
                        black_box(mt.transition_location(&loc));
                    }
                }));
            }
            for t in thrs {
                t.join().unwrap();
            }
        });
    }

    #[test]
    fn dont_trace_execution_of_a_trace() {
        let mt = Arc::new(MT::new().unwrap());
        mt.set_hot_threshold(0);
        let loc1 = Location::new();
        let loc2 = Location::new();

        // Get `loc1` to the point where there's a compiled trace for it.
        assert!(matches!(
            mt.transition_location(&loc1),
            TransitionLocation::StartTracing
        ));
        if let TransitionLocation::StopTracing(mtx) = mt.transition_location(&loc1) {
            *mtx.lock() = Some(Arc::new(unsafe { CompiledTrace::new_null() }));
        } else {
            panic!();
        }

        // If we transition `loc2` into `StartTracing`, then (for now) we should not execute the
        // trace for `loc1`, as another location is being traced and we don't want to trace the
        // execution of the trace!
        //
        // FIXME: this behaviour will need to change in the future:
        // https://github.com/ykjit/yk/issues/519
        assert!(matches!(
            mt.transition_location(&loc2),
            TransitionLocation::StartTracing
        ));
        assert!(matches!(
            mt.transition_location(&loc1),
            TransitionLocation::NoAction
        ));

        // But once we stop tracing for `loc2`, we should be able to execute the trace for `loc1`.
        assert!(matches!(
            mt.transition_location(&loc2),
            TransitionLocation::StopTracing(_)
        ));
        assert!(matches!(
            mt.transition_location(&loc1),
            TransitionLocation::Execute(_)
        ));
    }
}
