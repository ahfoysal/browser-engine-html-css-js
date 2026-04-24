//! QuickJS runtime + DOM glue.
//!
//! M6 upgrades the event model from a single `pending` FIFO to a proper
//! browser-style event loop:
//!
//! - **Microtask queue** — drained via `rt.execute_pending_job()` after
//!   every task. This is what makes `Promise.then` / `await` / chained
//!   `.then()` work: QuickJS reports promise callbacks as "pending jobs"
//!   and we pump them until the queue is empty.
//! - **Task queue with real time** — `setTimeout(cb, ms)` and
//!   `setInterval(cb, ms)` register into a `TimerQueue` keyed by an
//!   accumulated virtual clock. `drain_tasks` advances the clock to the
//!   earliest-due timer and runs it (plus its microtasks), repeating
//!   until nothing is due within `max_virtual_ms` of start.
//! - **`queueMicrotask`** — exposed directly via `Promise.resolve().then`.
//! - **`fetch(url)`** — returns a Promise; the body is fetched via the
//!   Rust-side `Fetcher` and the resolution is queued as a microtask so
//!   `.then(r => r.text())` resolves on the next drain.

use std::cell::RefCell;
use std::rc::Rc;

use rquickjs::{
    function::Func, Context, Ctx, Function, Persistent, Runtime, Value,
};

use super::dom::{Dom, NodeId};
use crate::net::Fetcher;

#[derive(Default)]
pub struct EventTable {
    /// (node, event name, callback)
    pub listeners: Vec<(NodeId, String, Persistent<Function<'static>>)>,
}

#[derive(Debug)]
pub struct Timer {
    pub id: u32,
    pub due_ms: u64,
    pub interval_ms: Option<u64>,
    pub callback: Persistent<Function<'static>>,
}

#[derive(Default)]
pub struct TimerQueue {
    pub next_id: u32,
    pub now_ms: u64,
    pub timers: Vec<Timer>,
    pub cancelled: Vec<u32>,
}

impl TimerQueue {
    fn earliest(&self) -> Option<usize> {
        self.timers
            .iter()
            .enumerate()
            .filter(|(_, t)| !self.cancelled.contains(&t.id))
            .min_by_key(|(_, t)| t.due_ms)
            .map(|(i, _)| i)
    }
}

#[derive(Default)]
pub struct ConsoleLog {
    pub lines: Vec<String>,
}

pub struct JsRuntime {
    pub(crate) rt: Runtime,
    ctx: Context,
    pub dom: Rc<RefCell<Dom>>,
    pub events: Rc<RefCell<EventTable>>,
    pub timers: Rc<RefCell<TimerQueue>>,
    pub console: Rc<RefCell<ConsoleLog>>,
    pub fetcher: Option<Rc<Fetcher>>,
}

/// JS shim that wraps raw node ids into browser-style `Element` objects
/// exposing `textContent`, `style`, `addEventListener`, `getAttribute`, etc.
/// M6 additions: `fetch()`, `queueMicrotask`, `clearTimeout`, `setInterval`.
const SHIM: &str = r#"
globalThis.__wrap = function(nid) {
    if (nid < 0 || nid === null || nid === undefined) return null;
    return {
        __nid: nid,
        get textContent() { return __getText(nid); },
        set textContent(v) { __setText(nid, String(v)); },
        getAttribute(k) { return __getAttr(nid, k); },
        setAttribute(k, v) { __setAttr(nid, k, String(v)); },
        addEventListener(type, fn) { __addListener(nid, String(type), fn); },
        style: new Proxy({}, {
            set(_t, k, v) { __setStyle(nid, String(k), String(v)); return true; },
            get(_t, _k) { return undefined; }
        })
    };
};
globalThis.document = {
    getElementById(id) { return __wrap(__getElementById(String(id))); },
    querySelector(sel) { return __wrap(__querySelector(String(sel))); }
};
globalThis.console = {
    log(...args) { __log(args.map(a => {
        try { return typeof a === 'string' ? a : JSON.stringify(a); }
        catch (_) { return String(a); }
    }).join(' ')); }
};

// --- M6: event-loop primitives -------------------------------------------
globalThis.queueMicrotask = function(cb) {
    Promise.resolve().then(() => { try { cb(); } catch(e) { __log('[microtask error] ' + e); } });
};

// `fetch(url)` returns a Promise<Response>. We call the sync Rust fetcher
// inside a microtask so the caller's .then chain is hooked up first.
globalThis.fetch = function(url, _opts) {
    return new Promise((resolve, reject) => {
        queueMicrotask(() => {
            try {
                const body = __fetchSync(String(url));
                if (body == null) {
                    reject(new Error('fetch failed: ' + url));
                    return;
                }
                const resp = {
                    ok: true,
                    status: 200,
                    url: String(url),
                    text() { return Promise.resolve(body); },
                    json() { return Promise.resolve(JSON.parse(body)); }
                };
                resolve(resp);
            } catch (e) {
                reject(e);
            }
        });
    });
};
"#;

impl JsRuntime {
    pub fn new(dom: Dom) -> rquickjs::Result<Self> {
        Self::new_with_fetcher(dom, None)
    }

    pub fn new_with_fetcher(dom: Dom, fetcher: Option<Rc<Fetcher>>) -> rquickjs::Result<Self> {
        let rt = Runtime::new()?;
        let ctx = Context::full(&rt)?;
        let dom = Rc::new(RefCell::new(dom));
        let events: Rc<RefCell<EventTable>> = Rc::new(RefCell::new(EventTable::default()));
        let timers: Rc<RefCell<TimerQueue>> = Rc::new(RefCell::new(TimerQueue::default()));
        let console: Rc<RefCell<ConsoleLog>> = Rc::new(RefCell::new(ConsoleLog::default()));

        ctx.with(|ctx| -> rquickjs::Result<()> {
            let globals = ctx.globals();

            // --- DOM accessors ---
            {
                let dom = dom.clone();
                globals.set(
                    "__getElementById",
                    Func::from(move |id: String| -> i32 {
                        dom.borrow().get_by_id(&id).map(|x| x as i32).unwrap_or(-1)
                    }),
                )?;
            }
            {
                let dom = dom.clone();
                globals.set(
                    "__querySelector",
                    Func::from(move |sel: String| -> i32 {
                        dom.borrow().query_selector(&sel).map(|x| x as i32).unwrap_or(-1)
                    }),
                )?;
            }
            {
                let dom = dom.clone();
                globals.set(
                    "__getText",
                    Func::from(move |id: i32| -> String {
                        if id < 0 { return String::new(); }
                        dom.borrow().text_content(id as NodeId)
                    }),
                )?;
            }
            {
                let dom = dom.clone();
                globals.set(
                    "__setText",
                    Func::from(move |id: i32, text: String| {
                        if id >= 0 {
                            dom.borrow_mut().set_text_content(id as NodeId, &text);
                        }
                    }),
                )?;
            }
            {
                let dom = dom.clone();
                globals.set(
                    "__getAttr",
                    Func::from(move |id: i32, name: String| -> Option<String> {
                        if id < 0 { return None; }
                        dom.borrow().get_attr(id as NodeId, &name)
                    }),
                )?;
            }
            {
                let dom = dom.clone();
                globals.set(
                    "__setAttr",
                    Func::from(move |id: i32, name: String, value: String| {
                        if id >= 0 {
                            dom.borrow_mut().set_attr(id as NodeId, &name, &value);
                        }
                    }),
                )?;
            }
            {
                let dom = dom.clone();
                globals.set(
                    "__setStyle",
                    Func::from(move |id: i32, prop: String, value: String| {
                        if id >= 0 {
                            dom.borrow_mut().set_style(id as NodeId, &prop, &value);
                        }
                    }),
                )?;
            }

            // --- events ---
            {
                let events = events.clone();
                globals.set(
                    "__addListener",
                    Func::from(move |ctx, id: i32, ty: String, cb| {
                        let ctx: Ctx = ctx;
                        let cb: Function = cb;
                        if id >= 0 {
                            let persistent: Persistent<Function<'static>> =
                                Persistent::save(&ctx, cb);
                            events
                                .borrow_mut()
                                .listeners
                                .push((id as NodeId, ty, persistent));
                        }
                    }),
                )?;
            }

            // --- timers (real event loop) ---
            {
                let timers = timers.clone();
                globals.set(
                    "setTimeout",
                    Func::from(move |ctx, cb, ms: Option<f64>| -> u32 {
                        let ctx: Ctx = ctx;
                        let cb: Function = cb;
                        let persistent: Persistent<Function<'static>> =
                            Persistent::save(&ctx, cb);
                        let mut q = timers.borrow_mut();
                        q.next_id += 1;
                        let id = q.next_id;
                        let delay = ms.unwrap_or(0.0).max(0.0) as u64;
                        let due = q.now_ms + delay;
                        q.timers.push(Timer {
                            id,
                            due_ms: due,
                            interval_ms: None,
                            callback: persistent,
                        });
                        id
                    }),
                )?;
            }
            {
                let timers = timers.clone();
                globals.set(
                    "setInterval",
                    Func::from(move |ctx, cb, ms: Option<f64>| -> u32 {
                        let ctx: Ctx = ctx;
                        let cb: Function = cb;
                        let persistent: Persistent<Function<'static>> =
                            Persistent::save(&ctx, cb);
                        let mut q = timers.borrow_mut();
                        q.next_id += 1;
                        let id = q.next_id;
                        let delay = ms.unwrap_or(0.0).max(1.0) as u64;
                        let due = q.now_ms + delay;
                        q.timers.push(Timer {
                            id,
                            due_ms: due,
                            interval_ms: Some(delay),
                            callback: persistent,
                        });
                        id
                    }),
                )?;
            }
            {
                let timers = timers.clone();
                globals.set(
                    "clearTimeout",
                    Func::from(move |id: u32| {
                        timers.borrow_mut().cancelled.push(id);
                    }),
                )?;
            }
            {
                let timers = timers.clone();
                globals.set(
                    "clearInterval",
                    Func::from(move |id: u32| {
                        timers.borrow_mut().cancelled.push(id);
                    }),
                )?;
            }

            // --- fetch (sync under the hood, wrapped as Promise in SHIM) ---
            if let Some(f) = &fetcher {
                let fclone = f.clone();
                globals.set(
                    "__fetchSync",
                    Func::from(move |url: String| -> Option<String> {
                        match fclone.fetch_text(&url) {
                            Ok(fetched) => Some(fetched.as_text()),
                            Err(e) => {
                                eprintln!("[js fetch] {}: {}", url, e);
                                None
                            }
                        }
                    }),
                )?;
            } else {
                globals.set(
                    "__fetchSync",
                    Func::from(move |_url: String| -> Option<String> {
                        eprintln!("[js fetch] no network: running offline");
                        None
                    }),
                )?;
            }

            // --- console ---
            {
                let console = console.clone();
                globals.set(
                    "__log",
                    Func::from(move |s: String| {
                        console.borrow_mut().lines.push(s);
                    }),
                )?;
            }

            // Install the JS-side wrappers.
            ctx.eval::<(), _>(SHIM)?;
            Ok(())
        })?;

        Ok(JsRuntime {
            rt,
            ctx,
            dom,
            events,
            timers,
            console,
            fetcher,
        })
    }

    /// Evaluate a chunk of page JS.
    pub fn eval(&self, source: &str) -> rquickjs::Result<()> {
        self.ctx.with(|ctx| -> rquickjs::Result<()> {
            ctx.eval::<(), _>(source)?;
            Ok(())
        })?;
        self.drain_microtasks();
        Ok(())
    }

    /// Drain the QuickJS promise/microtask job queue to completion.
    pub fn drain_microtasks(&self) {
        loop {
            match self.rt.execute_pending_job() {
                Ok(true) => continue,
                Ok(false) => break,
                Err(e) => {
                    eprintln!("[js] microtask error: {:?}", e);
                    break;
                }
            }
        }
    }

    /// Drive the event loop up to `max_virtual_ms` of simulated time.
    /// Runs all due timers in deadline order, draining microtasks after
    /// each one. This is how `setTimeout(..., 500)` + `fetch().then(...)`
    /// actually complete.
    pub fn drain_tasks(&self, max_virtual_ms: u64) -> rquickjs::Result<()> {
        // Microtasks from the initial eval first.
        self.drain_microtasks();

        loop {
            let (id_opt, due, cb_opt, interval) = {
                let q = self.timers.borrow();
                match q.earliest() {
                    None => (None, 0u64, None, None),
                    Some(i) => {
                        let t = &q.timers[i];
                        (Some(i), t.due_ms, Some(t.callback.clone()), t.interval_ms)
                    }
                }
            };
            let Some(idx) = id_opt else { break; };
            if due > max_virtual_ms {
                break;
            }

            // Remove (or reschedule for intervals) before firing.
            let fired_id = {
                let mut q = self.timers.borrow_mut();
                q.now_ms = due;
                let t = q.timers.remove(idx);
                if let Some(period) = interval {
                    let next_due = due + period.max(1);
                    q.timers.push(Timer {
                        id: t.id,
                        due_ms: next_due,
                        interval_ms: Some(period),
                        callback: t.callback.clone(),
                    });
                }
                t.id
            };

            // Skip cancelled.
            let cancelled = self.timers.borrow().cancelled.contains(&fired_id);
            if cancelled {
                continue;
            }

            if let Some(cb) = cb_opt {
                self.ctx.with(|ctx| -> rquickjs::Result<()> {
                    let f = cb.restore(&ctx)?;
                    let _: Value = f.call(())?;
                    Ok(())
                })?;
            }
            self.drain_microtasks();
        }
        Ok(())
    }

    /// Legacy name kept so main.rs / older docs compile. Advances 2s of
    /// virtual time, enough for the M6 sample's 500ms tick.
    pub fn drain_timers(&self) -> rquickjs::Result<()> {
        self.drain_tasks(2000)
    }

    /// Dispatch a synthetic event to all listeners registered on `target`
    /// for `event_name`. New listeners added during dispatch won't fire
    /// in this pass (they'd be part of the next event).
    pub fn dispatch_event(&self, target: NodeId, event_name: &str) -> rquickjs::Result<usize> {
        let snapshot: Vec<Persistent<Function<'static>>> = self
            .events
            .borrow()
            .listeners
            .iter()
            .filter(|(n, t, _)| *n == target && t == event_name)
            .map(|(_, _, f)| f.clone())
            .collect();
        let fired = snapshot.len();
        for cb in snapshot {
            self.ctx.with(|ctx| -> rquickjs::Result<()> {
                let f = cb.restore(&ctx)?;
                // Pass a minimal event-like object with `.type` + `.target.__nid`.
                let ev_src = format!(
                    "({{ type: '{}', target: __wrap({}) }})",
                    event_name.replace('\'', "\\'"),
                    target
                );
                let ev: Value = ctx.eval(ev_src.as_bytes())?;
                let _: Value = f.call((ev,))?;
                Ok(())
            })?;
        }
        self.drain_microtasks();
        Ok(fired)
    }

    pub fn dom_snapshot(&self) -> Dom {
        self.dom.borrow().clone_shallow()
    }
}

impl Drop for JsRuntime {
    fn drop(&mut self) {
        // QuickJS asserts on exit if any `Persistent` value outlives its
        // runtime without being explicitly restored + dropped inside the
        // context. We saved persistents into timers + listeners — take
        // them out and let them go inside `ctx.with` so the runtime sees
        // a clean GC root list.
        let timers: Vec<Persistent<Function<'static>>> = self
            .timers
            .borrow_mut()
            .timers
            .drain(..)
            .map(|t| t.callback)
            .collect();
        let listeners: Vec<Persistent<Function<'static>>> = self
            .events
            .borrow_mut()
            .listeners
            .drain(..)
            .map(|(_, _, f)| f)
            .collect();
        let _ = self.ctx.with(|ctx| -> rquickjs::Result<()> {
            for p in timers {
                if let Ok(f) = p.restore(&ctx) {
                    drop(f);
                }
            }
            for p in listeners {
                if let Ok(f) = p.restore(&ctx) {
                    drop(f);
                }
            }
            Ok(())
        });
    }
}

impl Dom {
    pub fn clone_shallow(&self) -> Dom {
        Dom {
            nodes: self.nodes.clone(),
            root: self.root,
            scripts: self.scripts.clone(),
        }
    }
}
