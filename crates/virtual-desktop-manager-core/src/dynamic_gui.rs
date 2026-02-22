//! Compose multiple [`nwg::PartialUi`] types in a convenient way.

/// A macro that generates a method that calls a method with the same name on
/// the dynamic UI, provided a [`DynamicUiRef`] is stored inside the current
/// type.
macro_rules! _forward_to_dynamic_ui {
    ($dynamic_ui:ident => $($method_name:ident),* $(,)?) => {
        $(
            fn $method_name(&self) {
                let Some($dynamic_ui) = self.$dynamic_ui.get() else { return };
                $dynamic_ui.$method_name();
            }
        )*
    }
}

#[allow(unused_imports)]
pub(crate) use _forward_to_dynamic_ui as forward_to_dynamic_ui;

use std::{
    any::{self, TypeId},
    cell::{Cell, OnceCell, Ref, RefCell},
    collections::VecDeque,
    fmt,
    marker::PhantomData,
    ops::Deref,
    rc::{Rc, Weak},
};

use crate::nwg_ext::enum_child_windows;

/// A trait object safe version of [`nwg::PartialUi`].
pub trait PartialUiDyn {
    /// Forwards calls to [`nwg::PartialUi::build_partial`].
    fn build_partial_dyn(
        &mut self,
        parent: Option<nwg::ControlHandle>,
    ) -> Result<(), nwg::NwgError>;

    /// Forwards calls to [`nwg::PartialUi::process_event`].
    fn process_event_dyn(
        &self,
        _evt: nwg::Event,
        _evt_data: &nwg::EventData,
        _handle: nwg::ControlHandle,
    ) {
    }

    /// Forwards calls to [`nwg::PartialUi::handles`].
    fn handles_dyn(&self) -> Vec<&'_ nwg::ControlHandle> {
        vec![]
    }
}
impl<T> PartialUiDyn for T
where
    T: nwg::PartialUi,
{
    fn build_partial_dyn(
        &mut self,
        parent: Option<nwg::ControlHandle>,
    ) -> Result<(), nwg::NwgError> {
        <T as nwg::PartialUi>::build_partial(self, parent)
    }
    fn process_event_dyn(
        &self,
        evt: nwg::Event,
        evt_data: &nwg::EventData,
        handle: nwg::ControlHandle,
    ) {
        <T as nwg::PartialUi>::process_event(self, evt, evt_data, handle)
    }
    fn handles_dyn(&self) -> Vec<&'_ nwg::ControlHandle> {
        <T as nwg::PartialUi>::handles(self)
    }
}

/// Allow downcast for trait object.
pub trait AsAny {
    fn as_any(&self) -> &dyn any::Any;
    fn type_name(&self) -> &'static str;
    /// Swap 2 values. Returns `true` if both values had the same type and so
    /// the swap was successful.
    fn swap_dyn(&mut self, other: &mut dyn any::Any) -> bool;
}
impl<T> AsAny for T
where
    T: Sized + 'static,
{
    fn as_any(&self) -> &dyn any::Any {
        self
    }
    #[cfg(debug_assertions)]
    fn type_name(&self) -> &'static str {
        any::type_name::<T>()
    }
    #[cfg(not(debug_assertions))]
    fn type_name(&self) -> &'static str {
        any::type_name::<dyn AsAny>()
    }
    fn swap_dyn(&mut self, other: &mut dyn any::Any) -> bool {
        if let Some(other) = <dyn any::Any>::downcast_mut::<T>(other) {
            core::mem::swap(self, other);
            true
        } else {
            false
        }
    }
}

pub trait DynWithDefault: AsAny {
    /// Create a temporary default value of the current type and provide it in a
    /// closure. The callback's first argument is `self` and the second argument
    /// is the new temporary default value. The callback can then modify the
    /// value as needed.
    fn with_default_mut(&mut self, f: &mut dyn FnMut(&mut dyn DynWithDefault, &mut dyn any::Any));

    /// Set `self` to a new default value and inspect the previous value as the
    /// second argument to the callback.
    fn clear_and_inspect_old(
        &mut self,
        f: &mut dyn FnMut(&mut dyn DynWithDefault, &mut dyn any::Any),
    ) {
        self.with_default_mut(&mut |current, new| {
            current.swap_dyn(new);
            let old = new;
            f(current, old);
        });
    }

    fn clear(&mut self) {
        self.with_default_mut(&mut |current, new| {
            current.swap_dyn(new);
        });
    }
}
impl<T> DynWithDefault for T
where
    T: Default + AsAny + 'static,
{
    fn with_default_mut(&mut self, f: &mut dyn FnMut(&mut dyn DynWithDefault, &mut dyn any::Any)) {
        f(self, &mut T::default())
    }
    fn clear_and_inspect_old(
        &mut self,
        f: &mut dyn FnMut(&mut dyn DynWithDefault, &mut dyn any::Any),
    ) {
        let mut old = core::mem::take(self);
        f(self, &mut old);
    }
    fn clear(&mut self) {
        *self = T::default();
    }
}

/// A trait for [`nwg::PartialUi`] types that wants to be managed by
/// [`DynamicUi`].
///
/// # Lifecycle
///
/// ## Initial build
///
/// For each item (in their specified order) these functions are called:
///
/// 1. [`DynamicUiHooks::before_partial_build`].
///    - If the `should_build: &mut bool` argument is set to `false` then all
///      steps until the `Rebuild` section is skipped. So no events will be
///      delivered to items that haven't been built. Such items will also be
///      skipped by helper methods like [`DynamicUi::for_each_ui`].
/// 2. [`PartialUiDyn::build_partial_dyn`].
/// 3. [`DynamicUiHooks::after_partial_build`].
/// 4. [`PartialUiDyn::handles_dyn`].
/// 5. [`DynamicUiHooks::after_handles`].
///    - The handles are used to bind event handlers.
/// 6. [`DynamicUiHooks::need_raw_events_for_children`].
///
/// ## Process events
///
/// 7. [`DynamicUiHooks::process_raw_event`].
///    - This might be called after [`DynamicUiHooks::after_process_events`]
///      depending on if the last registered event handler gets events first.
///      (The raw event handler is registered last.)
/// 8. [`PartialUiDyn::process_event_dyn`].
/// 9. [`DynamicUiHooks::after_process_events`].
///
/// ## Rebuild
///
/// After processing events the [`DynamicUi`] checks if there are items that
/// need to be rebuilt:
///
/// 10. [`DynamicUiHooks::need_rebuild`].
/// 11. [`DynamicUiHooks::is_ordered_in_parent`] if not rebuild was needed but a
///     previous sibling was rebuilt.
///
/// If one of the previous predicate functions returned `true` or the item's
/// parent was rebuilt then the item will be rebuilt:
///
/// 12. [`PartialUiDyn::handles_dyn`].
/// 13. [`DynamicUiHooks::after_handles`].
///     - The handles are used to unbind event handlers.
/// 15. [`DynamicUiHooks::before_rebuild`]
///
/// After that the same functions as the initial build is used.
pub trait DynamicUiHooks<T: ?Sized>: PartialUiDyn + DynWithDefault + 'static {
    /// Called before the item has been built. The returned parent will be
    /// passed to [`nwg::PartialUi::build_partial`] and used by controls in
    /// structs that make use of the [`nwd::NwgPartial`] derive macro.
    ///
    /// This function should also return the type ID of the dynamic ui that owns
    /// the control handle so that this ui is rebuilt in case its parent is
    /// rebuilt.
    ///
    /// The `&mut bool` argument can be set to `false` in order to not build
    /// this item.
    fn before_partial_build(
        &mut self,
        _dynamic_ui: &Rc<T>,
        _should_build: &mut bool,
    ) -> Option<(nwg::ControlHandle, TypeId)>;

    /// Called right after the ui has finished building.
    ///
    /// Note: this will also be called after the ui has been rebuilt.
    fn after_partial_build(&mut self, _dynamic_ui: &Rc<T>) {}

    /// Run right after [`nwg::PartialUi::handles`] and allows modifying its
    /// result.
    fn after_handles<'a>(
        &'a self,
        _dynamic_ui: &Rc<T>,
        _handles: &mut Vec<&'a nwg::ControlHandle>,
    ) {
    }

    /// Called after [`DynamicUiHooks::after_handles`] to check if we should
    /// bind **raw** event handlers for child controls as well.
    fn need_raw_events_for_children(&self) -> bool {
        false
    }

    /// Run right after [`nwg::PartialUi::process_event`] and allows easily
    /// doing some extra processing. Useful since the original method might be
    /// implemented by a derive macro which would make it difficult to modify.
    fn after_process_events(
        &self,
        _dynamic_ui: &Rc<T>,
        _evt: nwg::Event,
        _evt_data: &nwg::EventData,
        _handle: nwg::ControlHandle,
        _window: nwg::ControlHandle,
    ) {
    }
    /// Listen to raw window events (not filtered or processed by
    /// [`native_windows_gui`]). The first result that returns `Some` will be
    /// used as the actual return value for the event.
    ///
    /// Note that [`nwg::bind_raw_event_handler`] only listens for events on the
    /// top most control and not its children which differs from how
    /// [`nwg::full_bind_event_handler`] works and therefore you might see less
    /// events in this hook than in [`DynamicUiHooks::after_process_events`].
    fn process_raw_event(
        &self,
        _dynamic_ui: &Rc<T>,
        _hwnd: isize,
        _msg: u32,
        _w: usize,
        _l: isize,
        _window: nwg::ControlHandle,
    ) -> Option<isize> {
        None
    }

    /// Indicate that this item needs to be rebuilt. Maybe because its part of a
    /// context menu and its items need to be changed.
    ///
    /// This method is called automatically after
    /// [`DynamicUiHooks::after_process_events`] to check if processing the
    /// events changed the UI so that it needs to be rebuilt.
    fn need_rebuild(&self, _dynamic_ui: &Rc<T>) -> bool {
        false
    }
    /// Indicates if this item has a specific position relative to other items
    /// in its parent. If this returns `true` then the item will be rebuilt
    /// after previous siblings (items that share the same parent) are rebuilt.
    ///
    /// Defaults to `true` since its usually safer to rebuild more often.
    fn is_ordered_in_parent(&self) -> bool {
        true
    }
    /// Do some cleanup before the plugin is built again. By default this resets
    /// the state to its default value.
    fn before_rebuild(&mut self, _dynamic_ui: &Rc<T>) {
        self.clear();
    }
}

pub trait DynamicUiWrapper: Sized + 'static {
    type Hooks: ?Sized + DynamicUiHooks<Self>;

    fn get_dynamic_ui(&self) -> &DynamicUi<Self>;
    fn get_dynamic_ui_mut(&mut self) -> &mut DynamicUi<Self>;
}

/// A weak reference to the system tray. Equivalent to
/// `OnceCell<Weak<SystemTray>>`.
pub struct DynamicUiRef<T>(OnceCell<Weak<T>>);
impl<T> DynamicUiRef<T> {
    pub const fn new() -> Self {
        Self(OnceCell::new())
    }
    pub fn set(&self, dynamic_ui: &Rc<T>) {
        let _ = self.0.set(Rc::downgrade(dynamic_ui));
    }
    pub fn is_set(&self) -> bool {
        self.0.get().map_or(false, |tray| tray.strong_count() > 0)
    }
    pub fn get(&self) -> Option<Rc<T>> {
        self.0.get().and_then(Weak::upgrade)
    }
}
impl<T> fmt::Debug for DynamicUiRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DynamicUiRef").field(&self.0).finish()
    }
}
impl<T> Clone for DynamicUiRef<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Default for DynamicUiRef<T> {
    fn default() -> Self {
        Self(OnceCell::new())
    }
}
impl<T> From<&'_ Rc<T>> for DynamicUiRef<T> {
    fn from(dynamic_ui: &Rc<T>) -> Self {
        let this = Self::new();
        this.set(dynamic_ui);
        this
    }
}

/// This is mostly used to ensure no action is being executed while some partial
/// ui is being borrowed mutably.
struct DelayEventsGuard<'a>(&'a Cell<bool>);
impl<'a> DelayEventsGuard<'a> {
    fn new(delay_events: &'a Cell<bool>) -> Self {
        delay_events.set(true);
        Self(delay_events)
    }
}
impl Drop for DelayEventsGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginState {
    Destroyed,
    Built,
}

/// Data about a plugin kept by [`DynamicUi`]
struct PluginData<T: DynamicUiWrapper> {
    ui: Box<T::Hooks>,
    /// `None` if root item.
    parent_id: Option<TypeId>,
    /// Tracks if the item is destroyed or built.
    state: PluginState,
}
impl<T: DynamicUiWrapper> PluginData<T> {
    fn new(ui: Box<T::Hooks>) -> Self {
        Self {
            ui,
            parent_id: None,
            state: PluginState::Destroyed,
        }
    }
    fn after_build(ui: Box<T::Hooks>, parent_id: Option<TypeId>) -> Self {
        Self {
            ui,
            parent_id,
            state: PluginState::Built,
        }
    }
    fn id(&self) -> TypeId {
        <T::Hooks as AsAny>::as_any(&*self.ui).type_id()
    }
    fn plugin_type_name(&self) -> &'static str {
        <T::Hooks as AsAny>::type_name(&*self.ui)
    }
}
enum RawEventHandlerData {
    WithChildren(Vec<nwg::RawEventHandler>),
    ParentOnly(nwg::RawEventHandler),
    FailedToBind,
}
impl RawEventHandlerData {
    fn as_slice(&self) -> &[nwg::RawEventHandler] {
        match self {
            RawEventHandlerData::WithChildren(handlers) => handlers.as_slice(),
            RawEventHandlerData::ParentOnly(parent) => core::array::from_ref(parent),
            RawEventHandlerData::FailedToBind => &[],
        }
    }
}
struct EventHandlerData {
    plugin_id: TypeId,
    window: nwg::ControlHandle,
    handler: nwg::EventHandler,
    raw_handler: RawEventHandlerData,
}
impl Drop for EventHandlerData {
    fn drop(&mut self) {
        nwg::unbind_event_handler(&self.handler);
        for raw_handler in self.raw_handler.as_slice() {
            let _ = nwg::unbind_raw_event_handler(raw_handler);
        }
    }
}

/// Stores many [`nwg::PartialUi`] type. Usually behind an [`Rc`] pointer stored
/// inside [`DynamicUiRef`].
pub struct DynamicUi<T: DynamicUiWrapper> {
    /// Can be mutably borrowed when rebuilding. The type id indicates which
    /// plugin owns the parent control.
    ///
    /// # Lock pattern
    ///
    /// For write locks:
    ///
    /// 1. Check if [`Self::delay_events`] is set and if so don't acquire a
    ///    lock.
    /// 2. Create a [`DelayEventsGuard`] that sets the [`Self::delay_events`]
    ///    field to `true` (and then to `false` when dropped)
    /// 3. Acquiring the [`std::cell::RefMut`] guard.
    /// 4. Don't call into plugin hooks while holding the guard.
    ///
    /// For read locks:
    ///
    /// These should be okay everywhere except in functions that use
    /// [`DelayEventsGuard`].
    ui_list: RefCell<Vec<PluginData<T>>>,

    /// Used to store events that should be handled after rebuilding some plugins.
    #[allow(clippy::type_complexity)]
    event_queue: RefCell<VecDeque<Box<dyn FnOnce(&Rc<T>)>>>,

    /// Used to delay events while rebuilding. We take mutable references to
    /// plugins while rebuilding so this prevents issues where events are
    /// handled recursively while building new UI elements.
    delay_events: Cell<bool>,

    /// `true` if the UI should be destroyed.
    should_destroy: Cell<bool>,

    /// Event handlers that are bound to plugin windows.
    ///
    /// Raw event handlers can fail to be registered in which case we ignore the
    /// error (this only happens if raw event handler was registered previously
    /// with the same id for the same window).
    event_handlers: RefCell<Vec<EventHandlerData>>,

    /// Prevent recursive event handling.
    prevent_recursive_events: Cell<bool>,

    self_wrapper_ty: PhantomData<T>,
}
impl<T: DynamicUiWrapper> Default for DynamicUi<T> {
    fn default() -> Self {
        Self {
            ui_list: Default::default(),
            event_queue: Default::default(),
            delay_events: Default::default(),
            should_destroy: Default::default(),
            event_handlers: Default::default(),
            prevent_recursive_events: Default::default(),
            self_wrapper_ty: Default::default(),
        }
    }
}
impl<T> fmt::Debug for DynamicUi<T>
where
    T: DynamicUiWrapper,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynamicUi")
            .field(
                "plugins",
                &self.ui_list.try_borrow().map_or_else(
                    |e| vec![e.to_string()],
                    |plugins| {
                        plugins
                            .iter()
                            .map(|p| <T::Hooks as AsAny>::type_name(&*p.ui).to_string())
                            .collect::<Vec<_>>()
                    },
                ),
            )
            .field(
                "event_queue_len",
                &self.event_queue.try_borrow().map(|q| q.len()).ok(),
            )
            .field("delay_events", &self.delay_events)
            .finish()
    }
}
impl<T> DynamicUi<T>
where
    T: DynamicUiWrapper,
{
    pub fn new(ui_list: Vec<Box<T::Hooks>>) -> Self {
        let mut ui_list: Vec<_> = ui_list.into_iter().map(|ui| PluginData::new(ui)).collect();
        ui_list.shrink_to_fit();
        Self {
            ui_list: RefCell::new(ui_list),
            event_queue: Default::default(),
            delay_events: Default::default(),
            should_destroy: Default::default(),
            event_handlers: Default::default(),
            prevent_recursive_events: Default::default(),
            self_wrapper_ty: Default::default(),
        }
    }

    pub fn set_prevent_recursive_events(&self, value: bool) {
        self.prevent_recursive_events.set(value);
    }

    /// Run some code while delaying other event handlers.
    pub fn with_paused_events<R>(&self, f: impl FnOnce() -> R) -> R {
        let _prevent_other_actions = DelayEventsGuard::new(&self.delay_events);
        f()
    }

    /// Get a reference to a ui item managed by this dynamic ui.
    ///
    /// Warning: the returned ui item might not have been built.
    pub fn get_ui<U: DynamicUiHooks<T>>(&self) -> Option<Ref<'_, U>> {
        // TODO: prevent getting item that will be rebuilt (maybe only get items
        // that are earlier in the list than the item that is currently being
        // built).

        let guard = self.ui_list.borrow();
        Ref::filter_map(guard, |guard| {
            guard
                .iter()
                .find_map(|p| <T::Hooks as AsAny>::as_any(&*p.ui).downcast_ref::<U>())
        })
        .ok()
    }
    pub fn with_all_ui<R>(&self, f: impl FnOnce(&mut dyn Iterator<Item = &T::Hooks>) -> R) -> R {
        // TODO: don't allow this call while rebuilding.
        f(&mut self
            .ui_list
            .borrow()
            .iter()
            .filter(|item| item.state == PluginState::Built)
            .map(|p| &*p.ui))
    }
    pub fn for_each_ui(&self, f: impl FnMut(&T::Hooks)) {
        // TODO: don't allow this call while rebuilding.
        self.ui_list
            .borrow()
            .iter()
            .filter(|item| item.state == PluginState::Built)
            .map(|p| &*p.ui)
            .for_each(f);
    }
    /// Preforms an action and rebuilds the UI if needed. The action will be
    /// skipped if the UI is currently being rebuilt.
    pub fn maybe_preform_action<R>(wrapper: &Rc<T>, action: impl FnOnce(&Rc<T>) -> R) -> Option<R> {
        let this = wrapper.get_dynamic_ui();
        if this.delay_events.get() {
            tracing::warn!("A UI action was not performed because the UI was being rebuilt");
            None
        } else {
            let mut state = Some(Err(action));
            Self::preform_action_and_maybe_rebuild(
                wrapper,
                Some(&mut |wrapper| {
                    if let Some(Err(action)) = state.take() {
                        state = Some(Ok(action(wrapper)));
                    }
                }),
            );
            Some(
                state
                    .expect("callback panicked")
                    .ok()
                    .expect("callback never called"),
            )
        }
    }
    /// Preforms an action and then rebuilds the UI if needed. The action will
    /// be queued if UI is currently being rebuilt.
    ///
    /// The action will be called with the `wrapper` and a `bool` that is `true`
    /// if the action was delayed.
    pub fn preform_action<R>(
        wrapper: &Rc<T>,
        action: impl FnOnce(&Rc<T>, bool) -> R + 'static,
    ) -> Option<R> {
        let this = wrapper.get_dynamic_ui();
        if this.delay_events.get() {
            this.event_queue
                .borrow_mut()
                .push_back(Box::new(move |wrapper| drop(action(wrapper, true))));
            None
        } else {
            let mut state = Some(Err(action));
            Self::preform_action_and_maybe_rebuild(
                wrapper,
                Some(&mut |wrapper| {
                    if let Some(Err(action)) = state.take() {
                        state = Some(Ok(action(wrapper, false)));
                    }
                }),
            );
            Some(
                state
                    .expect("callback panicked")
                    .ok()
                    .expect("callback never called"),
            )
        }
    }
    pub fn preform_action_adv<S>(
        wrapper: &Rc<T>,
        state: S,
        delay_action: impl FnOnce(S) -> Option<Box<dyn FnOnce(&Rc<T>) + 'static>>,
        preform_action: impl FnOnce(S),
    ) {
        let this = wrapper.get_dynamic_ui();
        if this.delay_events.get() {
            if let Some(delayed) = delay_action(state) {
                this.event_queue.borrow_mut().push_back(delayed);
            }
        } else {
            let mut state = Some((state, preform_action));
            Self::preform_action_and_maybe_rebuild(
                wrapper,
                Some(&mut |_wrapper| {
                    if let Some((state, preform_action)) = state.take() {
                        preform_action(state);
                    }
                }),
            );
        }
    }
    #[allow(clippy::type_complexity)]
    fn preform_action_and_maybe_rebuild(
        wrapper: &Rc<T>,
        mut action: Option<&mut dyn FnMut(&Rc<T>)>,
    ) {
        let this = wrapper.get_dynamic_ui();
        if this.delay_events.get() {
            return;
        }

        loop {
            if this.should_destroy.get() {
                Self::destroy_ui(wrapper);
                return;
            }

            let first_queued = this.event_queue.borrow_mut().pop_front();
            if let Some(queued) = first_queued {
                queued(wrapper);
                continue;
            } else if let Some(action) = action.take() {
                action(wrapper);
            } else {
                // No action => handling events after rebuild => ensure no
                // plugin is invalidated
            }

            // Check for invalidated plugins:
            let mut rebuild_ids = VecDeque::new();
            for item in &*this.ui_list.borrow() {
                if item.ui.need_rebuild(wrapper) {
                    rebuild_ids.push_back(item.id());
                    tracing::info!("Dynamic ui required rebuild: {}", item.plugin_type_name());
                }
            }
            if rebuild_ids.is_empty() {
                return;
            }

            // Rebuild partial UIs:
            let _prevent_other_actions = DelayEventsGuard::new(&this.delay_events);
            {
                let Ok(mut guard) = this.ui_list.try_borrow_mut() else {
                    // Someone else must already be rebuilding. (Should not
                    // happen since we always have a DelayEventsGuard while
                    // taking a RefMut lock.)
                    tracing::warn!(
                        "Failed to lock plugin list in DynamicUi, this should never happen"
                    );
                    return;
                };
                let len = guard.len();
                let mut affected_parents = Vec::new();
                while let Some(rebuild_id) = rebuild_ids.pop_front() {
                    affected_parents.clear();
                    for ix in 0..len {
                        let item = &guard[ix];
                        let plugin = &item.ui;
                        let parent_id = item.parent_id;
                        let plugin_id = item.id();

                        if !affected_parents.is_empty() {
                            // Already rebuilt the requested plugin, now
                            // checking for later siblings (items that share the
                            // same parent) which need to be rebuilt in order
                            // for the UI elements to remain in the same order.
                            if let Some(parent_id) = parent_id {
                                if !affected_parents.contains(&parent_id) {
                                    continue; // item's parent isn't affected
                                }
                            } else {
                                continue; // items without parents aren't affected
                            }
                            // Is sibling to the rebuilt item!
                            if !plugin.is_ordered_in_parent() {
                                // This item doesn't depend on its build order
                                // relative to other items in its parent.
                                continue;
                            }
                            // Affected by the rebuilt item => rebuild it now!
                            rebuild_ids.retain(|&id| id != plugin_id);
                        } else if plugin_id != rebuild_id {
                            continue; // Keep looking for first item to rebuild!
                        }
                        let prev_state = item.state;

                        // Temporarily remove the partial ui:
                        let mut plugin = guard.swap_remove(ix).ui;

                        // Build the partial ui:
                        drop(guard);
                        tracing::info!(
                            "Rebuilding dynamic ui: {}",
                            <T::Hooks as AsAny>::type_name(&*plugin)
                        );

                        if prev_state == PluginState::Built {
                            // Unbind any event handlers associated with
                            // top-level windows in this partial ui:
                            let mut handles = plugin.handles_dyn();
                            plugin.after_handles(wrapper, &mut handles);
                            Self::unbind_specific_event_handlers(wrapper, &handles);

                            plugin.before_rebuild(wrapper);
                        }
                        let mut should_build = true;
                        let parent = plugin.before_partial_build(wrapper, &mut should_build);
                        let (plugin_data, res) = if should_build {
                            let res = plugin.build_partial_dyn(parent.map(|p| p.0));
                            <T::Hooks as DynamicUiHooks<T>>::after_partial_build(
                                &mut plugin,
                                wrapper,
                            );

                            let parent_id = parent.map(|(_, id)| id);
                            // might need to rebuild later items in the new parent:
                            if let Some(parent_id) = parent_id {
                                if !affected_parents.contains(&parent_id) {
                                    affected_parents.push(parent_id);
                                }
                            }

                            (PluginData::after_build(plugin, parent_id), res)
                        } else {
                            (PluginData::new(plugin), Ok(()))
                        };

                        // Put partial ui back in list:
                        guard = this.ui_list.borrow_mut();
                        guard.push(plugin_data);
                        guard.swap(ix, len - 1);

                        // Log build errors:
                        if let Err(e) = res {
                            tracing::error!(
                                "Rebuild of dynamic ui {} failed: {e:?}",
                                guard[ix].plugin_type_name()
                            );
                        }

                        // Queue children for rebuild:
                        for child in &*guard {
                            // TODO: detect cycles (we could enforce that
                            // children are always after their parents in the
                            // plugin list)
                            if child.parent_id == Some(plugin_id) {
                                let id = child.id();
                                if !rebuild_ids.contains(&id) {
                                    rebuild_ids.push_back(id);
                                }
                            }
                        }

                        if affected_parents.is_empty() {
                            break; // Don't need to check if siblings need to be rebuilt
                        }
                    }
                }
            }

            // Rebind event handlers if window handles changed.
            Self::bind_event_handlers(wrapper);

            // Continue main loop since there might be new queued events.
        }
    }

    fn initial_build(wrapper: &Rc<T>) -> Result<(), nwg::NwgError> {
        {
            let this = wrapper.get_dynamic_ui();
            let ui_list = &this.ui_list;
            let mut guard = ui_list.borrow_mut();

            // Prevent rebuilds during initial build (this should not happen
            // anyway since no event handlers are registered until we return):
            let _prevent_other_actions = DelayEventsGuard::new(&this.delay_events);

            // Build plugins:
            let mut plugin_ix = 0;
            loop {
                let len = guard.len();
                if plugin_ix >= len {
                    break;
                }
                // Remove the partial ui from the list:
                let mut plugin = guard.swap_remove(plugin_ix).ui;

                // Build the partial ui:
                drop(guard);
                let mut should_build = true;
                let parent = plugin.before_partial_build(wrapper, &mut should_build);
                let (plugin, res) = if should_build {
                    let res = plugin.build_partial_dyn(parent.map(|p| p.0));
                    DynamicUiHooks::after_partial_build(&mut *plugin, wrapper);

                    let parent_id = parent.map(|(_, id)| id);
                    (PluginData::after_build(plugin, parent_id), res)
                } else {
                    (PluginData::new(plugin), Ok(()))
                };

                // Put the partial ui back where we took it:
                guard = ui_list.borrow_mut();
                guard.insert(len - 1, plugin);
                guard.swap(plugin_ix, len - 1);

                res?;

                plugin_ix += 1;
            }
        }

        Ok(())
    }
    fn all_handles(wrapper: &Rc<T>) -> Vec<(TypeId, bool, nwg::ControlHandle)> {
        wrapper
            .get_dynamic_ui()
            .ui_list
            .borrow()
            .iter()
            .filter(|item| item.state == PluginState::Built)
            .flat_map(|item| {
                // The derive macro `NwgPartial` always emits Vec::new(), so don't
                // expect anything here:
                let mut item_handles = item.ui.handles_dyn();
                // But plugins can easily add more handles:
                item.ui.after_handles(wrapper, &mut item_handles);

                let raw_child_handlers = item.ui.need_raw_events_for_children();

                // Remember what plugin a window is associated with:
                let id = item.id();
                item_handles
                    .into_iter()
                    .copied()
                    .map(move |handle| (id, raw_child_handlers, handle))
            })
            .collect()
    }
    fn process_events_for_plugin_and_children(
        wrapper: &Rc<T>,
        plugin_id: TypeId,
        mut f: impl FnMut(&PluginData<T>),
    ) {
        let this = wrapper.get_dynamic_ui();
        let _event_guard = this
            .prevent_recursive_events
            .get()
            .then(|| DelayEventsGuard::new(&this.delay_events));
        let guard = this.ui_list.borrow();
        let mut parent_ids = Vec::<TypeId>::with_capacity(guard.len());

        for item in &*guard {
            if item.state != PluginState::Built {
                continue;
            }
            if parent_ids.is_empty() {
                let id = item.id();
                if id != plugin_id {
                    continue; // Not the plugin that owns the window
                }
                parent_ids.push(id);
            } else if let Some(parent_id) = item.parent_id {
                if !parent_ids.contains(&parent_id) {
                    continue; // Doesn't have an affected parent
                } else {
                    parent_ids.push(item.id());
                }
            } else {
                continue; // No parent
            }

            f(item);
        }
    }
    fn process_event(
        wrapper: &Rc<T>,
        evt: nwg::Event,
        evt_data: &nwg::EventData,
        handle: nwg::ControlHandle,
        window: nwg::ControlHandle,
        plugin_id: TypeId,
    ) {
        if !matches!(
            evt,
            nwg::Event::OnNotice | nwg::Event::OnMouseMove | nwg::Event::Unknown
        ) {
            // Note: We use nwg::Notice for timers so they are triggered a lot.
            tracing::trace!(
                event = ?evt,
                event_data = ?evt_data,
                handle = ?handle,
                window = ?window,
                "SystemTrayUiAdaptor::process_event()"
            );
        }
        Self::process_events_for_plugin_and_children(wrapper, plugin_id, move |item| {
            item.ui.process_event_dyn(evt, evt_data, handle);
            item.ui
                .after_process_events(wrapper, evt, evt_data, handle, window);
        });
    }
    fn process_raw_event(
        wrapper: &Rc<T>,
        hwnd: isize,
        msg: u32,
        w: usize,
        l: isize,
        window: nwg::ControlHandle,
        plugin_id: TypeId,
    ) -> Option<isize> {
        let mut first = None;
        Self::process_events_for_plugin_and_children(wrapper, plugin_id, |item| {
            if let Some(result) = item.ui.process_raw_event(wrapper, hwnd, msg, w, l, window) {
                if let Some(first_result) = first {
                    tracing::warn!(
                        ?first_result,
                        ?result,
                        "Multiple raw event handlers returned a result, the first result will be used"
                    )
                } else {
                    first = Some(result);
                }
            }
        });
        first
    }
    fn unbind_specific_event_handlers(wrapper: &Rc<T>, window_handles: &[&nwg::ControlHandle]) {
        let this = wrapper.get_dynamic_ui();
        this.event_handlers
            .borrow_mut()
            .retain(|data| !window_handles.contains(&&data.window));
    }
    fn unbind_event_handlers(wrapper: &Rc<T>) {
        let this = wrapper.get_dynamic_ui();
        this.event_handlers.take();
    }
    fn bind_event_handlers(wrapper: &Rc<T>) {
        let this = wrapper.get_dynamic_ui();
        let window_handles = Self::all_handles(wrapper);
        let current_handles = this
            .event_handlers
            .borrow()
            .iter()
            .map(|data| {
                (
                    data.plugin_id,
                    matches!(data.raw_handler, RawEventHandlerData::WithChildren(_)),
                    data.window,
                )
            })
            .collect::<Vec<_>>();
        if window_handles == current_handles {
            return;
        }
        tracing::debug!(
            ?window_handles,
            previous_handles = ?current_handles,
            "Binding event handlers to windows"
        );

        Self::unbind_event_handlers(wrapper);

        let mut handlers = Vec::with_capacity(window_handles.len());
        for &(plugin_id, raw_child_events, window) in window_handles.iter() {
            // Note: bind raw event handler first so that nwg's event handler
            // doesn't suppress an event before we see it.
            let evt_ui = Rc::downgrade(wrapper);
            let handle_raw_events = move |hwnd, msg, l, w| {
                if let Some(ui) = evt_ui.upgrade() {
                    // !!! Partials Event Dispatch !!!
                    Self::preform_action(&ui, {
                        move |ui, delayed| {
                            let res = DynamicUi::process_raw_event(
                                ui,
                                hwnd as isize,
                                msg,
                                l,
                                w,
                                window,
                                plugin_id,
                            );
                            if let (Some(res), true) = (res, delayed) {
                                tracing::warn!(
                                    return_value = ?res,
                                    "Delayed handling of raw event and now can't handle return value"
                                );
                            }
                            res
                        }
                    })
                    .flatten()
                    .inspect(|val| {
                        tracing::debug!(
                            return_value = ?val,
                            "Returned custom value from raw event handle"
                        );
                    })
                } else {
                    None
                }
            };
            let raw_event_handler = match nwg::bind_raw_event_handler(
                &window,
                // This argument has to be > 0xFFFF, but otherwise can be anything:
                0x85dead,
                handle_raw_events.clone(),
            ) {
                Ok(event_handler) => {
                    if raw_child_events {
                        if let Some(hwnd) = window.hwnd() {
                            let mut handlers = vec![event_handler];
                            enum_child_windows(
                                Some(windows::Win32::Foundation::HWND(hwnd.cast())),
                                |child| {
                                    match nwg::bind_raw_event_handler(
                                        &window,
                                        // This argument has to be > 0xFFFF, but otherwise can be anything:
                                        0xc85dead,
                                        handle_raw_events.clone(),
                                    ) {
                                        Ok(handler) => handlers.push(handler),
                                        Err(e) => {
                                            tracing::warn!(?window, ?child, "Failed to register raw event handler for child window: {e}");
                                        }
                                    }
                                    std::ops::ControlFlow::Continue(())
                                },
                            );
                            RawEventHandlerData::WithChildren(handlers)
                        } else {
                            tracing::warn!("Could not find child windows since parent handle wasn't for a window");
                            RawEventHandlerData::ParentOnly(event_handler)
                        }
                    } else {
                        RawEventHandlerData::ParentOnly(event_handler)
                    }
                }
                Err(e) => {
                    tracing::warn!(?window, "Failed to register raw event handler: {e}");
                    RawEventHandlerData::FailedToBind
                }
            };

            let evt_ui = Rc::downgrade(wrapper);
            let handle_events = move |evt, evt_data, handle| {
                if let Some(ui) = evt_ui.upgrade() {
                    // !!! Partials Event Dispatch !!!
                    Self::preform_action(&ui, {
                        move |ui, _| {
                            DynamicUi::process_event(ui, evt, &evt_data, handle, window, plugin_id);
                        }
                    });
                }
            };
            let event_handler = nwg::full_bind_event_handler(&window, handle_events);

            handlers.push(EventHandlerData {
                plugin_id,
                window,
                handler: event_handler,
                raw_handler: raw_event_handler,
            });
        }

        *this.event_handlers.borrow_mut() = handlers;
    }
    /// To make sure that everything is freed without issues, the default
    /// handler must be unbound.
    fn destroy_ui(wrapper: &Rc<T>) {
        let this = wrapper.get_dynamic_ui();
        this.should_destroy.set(true);

        Self::unbind_event_handlers(wrapper);

        if this.delay_events.get() {
            return;
        }
        let _prevent_other_actions = DelayEventsGuard::new(&this.delay_events);
        for item in &mut *this.ui_list.borrow_mut() {
            if item.state == PluginState::Destroyed {
                continue;
            }
            item.ui.before_rebuild(wrapper);
            item.state = PluginState::Destroyed;
        }
    }
}

/// When this goes out of scope the [`DynamicUi`] will be destroyed.
pub struct DynamicUiOwner<T>(Rc<T>)
where
    T: DynamicUiWrapper;
impl<T> Deref for DynamicUiOwner<T>
where
    T: DynamicUiWrapper,
{
    type Target = Rc<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T> Drop for DynamicUiOwner<T>
where
    T: DynamicUiWrapper,
{
    fn drop(&mut self) {
        DynamicUi::destroy_ui(&self.0);
    }
}

/// Manual implementation of [`nwg::NativeUi`] for [`DynamicUi`] mostly because
/// the [`nwd::NwgUi`] derive macro ignores handles from
/// [`nwg::PartialUi::handles`].
///
/// # References
///
/// - [Native Windows GUI guide -
///   Partials](https://gabdube.github.io/native-windows-gui/native-windows-docs/partial.html)
/// - [native-windows-gui/native-windows-gui/examples/partials.rs at
///   a6c96e8de5d01fe7bb566d737622dfead3cd1aed ·
///   gabdube/native-windows-gui](https://github.com/gabdube/native-windows-gui/blob/a6c96e8de5d01fe7bb566d737622dfead3cd1aed/native-windows-gui/examples/partials.rs)
impl<T> nwg::NativeUi<DynamicUiOwner<T>> for Rc<T>
where
    T: DynamicUiWrapper,
{
    fn build_ui(data: Self) -> Result<DynamicUiOwner<T>, nwg::NwgError> {
        let data = DynamicUiOwner(data);
        DynamicUi::initial_build(&data.0)?;
        DynamicUi::bind_event_handlers(&data.0);
        Ok(data)
    }
}
