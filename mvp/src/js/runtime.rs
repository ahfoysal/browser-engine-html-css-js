//! QuickJS runtime + DOM glue.
//!
//! Everything JS-land talks to lives behind a `Rc<RefCell<Dom>>` plus a
//! `Rc<RefCell<EventTable>>` for listeners and a `Rc<RefCell<TimerQueue>>`
//! for `setTimeout`. The QuickJS context is single-threaded and so are we —
//! all callbacks run on the main thread between layout passes.

use std::cell::RefCell;
use std::rc::Rc;

use rquickjs::{
    function::Func, Context, Ctx, Function, Persistent, Runtime, Value,
};

use super::dom::{Dom, NodeId};

#[derive(Default)]
pub struct EventTable {
    /// (node, event name, callback)
    pub listeners: Vec<(NodeId, String, Persistent<Function<'static>>)>,
}

#[derive(Default)]
pub struct TimerQueue {
    /// Due-immediately queue: we don't model real time, timers just run in
    /// registration order after the initial script finishes.
    pub pending: Vec<Persistent<Function<'static>>>,
}

#[derive(Default)]
pub struct ConsoleLog {
    pub lines: Vec<String>,
}

pub struct JsRuntime {
    _rt: Runtime,
    ctx: Context,
    pub dom: Rc<RefCell<Dom>>,
    pub events: Rc<RefCell<EventTable>>,
    pub timers: Rc<RefCell<TimerQueue>>,
    pub console: Rc<RefCell<ConsoleLog>>,
}

/// JS shim that wraps raw node ids into browser-style `Element` objects
/// exposing `textContent`, `style`, `addEventListener`, `getAttribute`, etc.
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
"#;

impl JsRuntime {
    pub fn new(dom: Dom) -> rquickjs::Result<Self> {
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

            // --- timers ---
            {
                let timers = timers.clone();
                globals.set(
                    "setTimeout",
                    Func::from(move |ctx, cb, _ms: f64| -> i32 {
                        let ctx: Ctx = ctx;
                        let cb: Function = cb;
                        let persistent: Persistent<Function<'static>> =
                            Persistent::save(&ctx, cb);
                        timers.borrow_mut().pending.push(persistent);
                        0
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
            _rt: rt,
            ctx,
            dom,
            events,
            timers,
            console,
        })
    }

    /// Evaluate a chunk of page JS.
    pub fn eval(&self, source: &str) -> rquickjs::Result<()> {
        self.ctx.with(|ctx| -> rquickjs::Result<()> {
            ctx.eval::<(), _>(source)?;
            Ok(())
        })
    }

    /// Drain any pending `setTimeout` callbacks in registration order.
    pub fn drain_timers(&self) -> rquickjs::Result<()> {
        loop {
            let next = self.timers.borrow_mut().pending.drain(..).collect::<Vec<_>>();
            if next.is_empty() {
                break;
            }
            for cb in next {
                self.ctx.with(|ctx| -> rquickjs::Result<()> {
                    let f = cb.restore(&ctx)?;
                    let _: Value = f.call(())?;
                    Ok(())
                })?;
            }
        }
        Ok(())
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
        Ok(fired)
    }

    /// Take ownership of the mutated DOM for re-layout/paint. The runtime
    /// keeps its own `Rc` alive (JS may still reference it), but layout
    /// only needs a snapshot converted to `html::Node`.
    pub fn dom_snapshot(&self) -> Dom {
        // Clone via serialize + rebuild would be wasteful; we just clone
        // the arena. Scripts are only relevant once and we don't need them
        // again after the first eval.
        self.dom.borrow().clone_shallow()
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
