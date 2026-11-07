//! UI Automation provider for the main window: a fragment root for the
//! window and one element per accessible node (`a11y::Node`), with Invoke,
//! Toggle, ExpandCollapse and SelectionItem patterns.
//!
//! UIA calls the provider from its own threads, never the UI thread, so it
//! reads a published snapshot (`publish`, called by the window on its own
//! thread) and never the window's state. Actions are posted to the window
//! as `WM_UIA_ACT` and run on the UI thread like a click.
#![allow(unsafe_code)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use windows::{
    core::{implement, IUnknown, IUnknownImpl, Interface, BOOL, BSTR},
    Win32::{
        Foundation::{E_FAIL, E_INVALIDARG, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{ClientToScreen, ScreenToClient},
        System::{
            Com::SAFEARRAY,
            Ole::{SafeArrayCreateVector, SafeArrayPutElement},
            Variant::{VARIANT, VT_I4},
        },
        UI::{
            Accessibility::{
                ExpandCollapseState, ExpandCollapseState_Collapsed, ExpandCollapseState_Expanded,
                ExpandCollapseState_LeafNode, IExpandCollapseProvider,
                IExpandCollapseProvider_Impl, IInvokeProvider, IInvokeProvider_Impl,
                IRawElementProviderFragment, IRawElementProviderFragmentRoot,
                IRawElementProviderFragmentRoot_Impl, IRawElementProviderFragment_Impl,
                IRawElementProviderSimple, IRawElementProviderSimple_Impl, ISelectionItemProvider,
                ISelectionItemProvider_Impl, IToggleProvider, IToggleProvider_Impl,
                NavigateDirection, NavigateDirection_FirstChild, NavigateDirection_LastChild,
                NavigateDirection_NextSibling, NavigateDirection_Parent,
                NavigateDirection_PreviousSibling, ProviderOptions,
                ProviderOptions_ServerSideProvider, ToggleState, ToggleState_Off, ToggleState_On,
                UiaAppendRuntimeId, UiaHostProviderFromHwnd, UiaRect, UiaReturnRawElementProvider,
                UIA_PATTERN_ID, UIA_PROPERTY_ID,
            },
            WindowsAndMessaging::{GetClientRect, PostMessageW, WM_APP},
        },
    },
};

use crate::{a11y::Node, pages::Role};

/// Posted to the window by a pattern call: `wparam` = node id, `lparam` =
/// an [`Act`] code.
pub const WM_UIA_ACT: u32 = WM_APP + 0x40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Click, Enter: Invoke, Toggle, Expand/Collapse and Select all do this.
    Activate = 0,
    /// Move keyboard focus to the node.
    Focus = 1,
}

/// What UIA reads: the nodes as last painted and where focus is.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub nodes: Vec<Node>,
    pub focus: Option<u32>,
}

/// Each window's last-painted tree, keyed by its handle: a flyout and the main window publish their own.
static SNAPSHOTS: Mutex<Option<HashMap<isize, Arc<Snapshot>>>> = Mutex::new(None);

/// Replaces window `key`'s snapshot in `store`; true when it changed.
fn publish_into(store: &mut HashMap<isize, Arc<Snapshot>>, key: isize, s: Snapshot) -> bool {
    let same = store
        .get(&key)
        .is_some_and(|o| o.focus == s.focus && o.nodes == s.nodes);
    if !same {
        store.insert(key, Arc::new(s));
    }
    !same
}

/// The snapshot of window `hwnd` (empty when it has published none).
fn snapshot_for(hwnd: isize) -> Arc<Snapshot> {
    SNAPSHOTS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_or_insert_with(HashMap::new)
        .get(&hwnd)
        .cloned()
        .unwrap_or_default()
}

/// Replaces window `hwnd`'s snapshot; true when it changed.
pub fn publish(hwnd: isize, s: Snapshot) -> bool {
    let mut g = SNAPSHOTS.lock().unwrap_or_else(|p| p.into_inner());
    publish_into(g.get_or_insert_with(HashMap::new), hwnd, s)
}

/// Whether any UI Automation client is attached (building the tree is
/// skipped otherwise).
pub fn listening() -> bool {
    // SAFETY: plain query.
    unsafe { windows::Win32::UI::Accessibility::UiaClientsAreListening() }.as_bool()
}

/// The `UiaRootObjectId` of `WM_GETOBJECT`.
pub const ROOT_OBJECT_ID: i32 = -25;

/// The answer to `WM_GETOBJECT` for the window.
pub fn root_for(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let root: IRawElementProviderSimple = Elem {
        hwnd: hwnd.0 as isize,
        node: None,
    }
    .into();
    // SAFETY: hands UIA a provider; it takes its own reference.
    unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &root) }
}

/// Raises the focus-changed event for node `id`.
pub fn raise_focus(hwnd: HWND, id: u32) {
    if !listening() {
        return;
    }
    let el: IRawElementProviderSimple = Elem {
        hwnd: hwnd.0 as isize,
        node: Some(id),
    }
    .into();
    // SAFETY: UIA copies what it needs from the provider.
    unsafe {
        let _ =
            windows::Win32::UI::Accessibility::UiaRaiseAutomationEvent(&el, FOCUS_CHANGED_EVENT);
    }
}

const FOCUS_CHANGED_EVENT: windows::Win32::UI::Accessibility::UIA_EVENT_ID =
    windows::Win32::UI::Accessibility::UIA_EVENT_ID(20005);

// Property, pattern and control type ids (UIAutomationClient.h).
const P_NAME: i32 = 30005;
const P_CONTROL_TYPE: i32 = 30003;
const P_ENABLED: i32 = 30010;
const P_KEYBOARD_FOCUSABLE: i32 = 30009;
const P_HAS_FOCUS: i32 = 30008;
const P_AUTOMATION_ID: i32 = 30011;
const P_CLASS_NAME: i32 = 30012;
const P_IS_CONTROL: i32 = 30016;
const P_IS_CONTENT: i32 = 30017;
const P_OFFSCREEN: i32 = 30022;
const P_FRAMEWORK: i32 = 30024;
const PAT_INVOKE: i32 = 10000;
const PAT_EXPAND: i32 = 10005;
const PAT_SELECTION_ITEM: i32 = 10010;
const PAT_TOGGLE: i32 = 10015;

fn control_type(r: Role) -> i32 {
    match r {
        Role::Button => 50000,
        Role::Toggle => 50002,
        Role::Radio => 50013,
        Role::Header => 50024,
        Role::Item => 50007,
        Role::Tab => 50019,
    }
}

/// The window (`node` None) or one of its nodes, by focus id.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot,
    IInvokeProvider,
    IToggleProvider,
    IExpandCollapseProvider,
    ISelectionItemProvider
)]
struct Elem {
    hwnd: isize,
    node: Option<u32>,
}

impl Elem {
    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }

    fn sibling(&self, id: Option<u32>) -> Elem {
        Elem {
            hwnd: self.hwnd,
            node: id,
        }
    }

    fn node(&self, s: &Snapshot) -> Option<Node> {
        let id = self.node?;
        s.nodes.iter().find(|n| n.id == id).cloned()
    }

    fn post(&self, act: Act) -> windows::core::Result<()> {
        let id = self.node.ok_or(E_FAIL)?;
        // SAFETY: a message to the window; it checks the node itself.
        unsafe {
            PostMessageW(
                Some(self.hwnd()),
                WM_UIA_ACT,
                WPARAM(id as usize),
                LPARAM(act as isize),
            )
        }
    }

    fn client_rect(&self) -> RECT {
        let mut r = RECT::default();
        // SAFETY: reads the window's client rect (thread-safe).
        let _ = unsafe { GetClientRect(self.hwnd(), &mut r) };
        r
    }
}

fn provider(e: Elem) -> IRawElementProviderFragment {
    let p: IRawElementProviderFragment = e.into();
    p
}

fn variant_bool(b: bool) -> VARIANT {
    VARIANT::from(b)
}

fn variant_str(s: &str) -> VARIANT {
    VARIANT::from(BSTR::from(s))
}

#[allow(non_snake_case)]
impl IRawElementProviderSimple_Impl for Elem_Impl {
    fn ProviderOptions(&self) -> windows::core::Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }

    fn GetPatternProvider(&self, id: UIA_PATTERN_ID) -> windows::core::Result<IUnknown> {
        let s = snapshot_for(self.hwnd);
        let Some(n) = self.node(&s) else {
            return Err(E_FAIL.into());
        };
        let want = match id.0 {
            PAT_INVOKE => matches!(n.role, Role::Button),
            PAT_TOGGLE => n.toggled.is_some(),
            PAT_EXPAND => n.expanded.is_some(),
            PAT_SELECTION_ITEM => matches!(n.role, Role::Radio | Role::Tab),
            _ => false,
        };
        if !want {
            return Err(E_FAIL.into());
        }
        let this: IRawElementProviderSimple = self.to_interface();
        match id.0 {
            PAT_INVOKE => this.cast::<IInvokeProvider>()?.cast(),
            PAT_TOGGLE => this.cast::<IToggleProvider>()?.cast(),
            PAT_EXPAND => this.cast::<IExpandCollapseProvider>()?.cast(),
            _ => this.cast::<ISelectionItemProvider>()?.cast(),
        }
    }

    fn GetPropertyValue(&self, id: UIA_PROPERTY_ID) -> windows::core::Result<VARIANT> {
        let s = snapshot_for(self.hwnd);
        let Some(n) = self.node(&s) else {
            // The window itself: the host provider supplies most of it.
            return Ok(match id.0 {
                P_NAME => variant_str("Relay"),
                P_CONTROL_TYPE => VARIANT::from(50033_i32),
                P_FRAMEWORK => variant_str("Relay"),
                _ => VARIANT::default(),
            });
        };
        Ok(match id.0 {
            P_NAME => variant_str(&n.name),
            P_CONTROL_TYPE => VARIANT::from(control_type(n.role)),
            P_ENABLED | P_IS_CONTROL | P_IS_CONTENT | P_KEYBOARD_FOCUSABLE => variant_bool(true),
            P_HAS_FOCUS => variant_bool(s.focus == Some(n.id)),
            P_AUTOMATION_ID => variant_str(&format!("relay-{}", n.id)),
            P_CLASS_NAME => variant_str("RelayControl"),
            P_FRAMEWORK => variant_str("Relay"),
            P_OFFSCREEN => variant_bool(n.rect.bottom <= n.rect.top),
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> windows::core::Result<IRawElementProviderSimple> {
        if self.node.is_some() {
            return Err(E_FAIL.into());
        }
        // SAFETY: asks UIA for the window's default provider.
        unsafe { UiaHostProviderFromHwnd(self.hwnd()) }
    }
}

#[allow(non_snake_case)]
impl IRawElementProviderFragment_Impl for Elem_Impl {
    fn Navigate(
        &self,
        dir: NavigateDirection,
    ) -> windows::core::Result<IRawElementProviderFragment> {
        let s = snapshot_for(self.hwnd);
        let ids: Vec<u32> = s.nodes.iter().map(|n| n.id).collect();
        let target = match (self.node, dir) {
            (None, d) if d == NavigateDirection_FirstChild => ids.first().copied(),
            (None, d) if d == NavigateDirection_LastChild => ids.last().copied(),
            (None, _) => None,
            (Some(_), d) if d == NavigateDirection_Parent => {
                return Ok(provider(self.sibling(None)));
            }
            (Some(id), d) => {
                let at = ids.iter().position(|&x| x == id);
                match (at, d) {
                    (Some(i), d) if d == NavigateDirection_NextSibling => ids.get(i + 1).copied(),
                    (Some(i), d) if d == NavigateDirection_PreviousSibling => {
                        i.checked_sub(1).and_then(|j| ids.get(j).copied())
                    }
                    _ => None,
                }
            }
        };
        match target {
            Some(id) => Ok(provider(self.sibling(Some(id)))),
            // No such neighbour: UIA expects a null provider, not an error.
            None => Err(windows::core::Error::empty()),
        }
    }

    fn GetRuntimeId(&self) -> windows::core::Result<*mut SAFEARRAY> {
        let Some(id) = self.node else {
            return Ok(std::ptr::null_mut());
        };
        // SAFETY: a two-element I32 array filled with the documented prefix
        // and the node id; ownership passes to UIA.
        unsafe {
            let sa = SafeArrayCreateVector(VT_I4, 0, 2);
            if sa.is_null() {
                return Err(E_FAIL.into());
            }
            let parts = [UiaAppendRuntimeId as i32, id as i32];
            for (i, v) in parts.iter().enumerate() {
                SafeArrayPutElement(sa, &(i as i32), (v as *const i32).cast())?;
            }
            Ok(sa)
        }
    }

    fn BoundingRectangle(&self) -> windows::core::Result<UiaRect> {
        let s = snapshot_for(self.hwnd);
        let r = match self.node {
            None => {
                let c = self.client_rect();
                (c.left as f32, c.top as f32, c.right as f32, c.bottom as f32)
            }
            Some(_) => {
                let n = self.node(&s).ok_or(E_FAIL)?;
                (n.rect.left, n.rect.top, n.rect.right, n.rect.bottom)
            }
        };
        let mut p = POINT {
            x: r.0.round() as i32,
            y: r.1.round() as i32,
        };
        // SAFETY: maps a client point to the screen (thread-safe).
        unsafe {
            let _ = ClientToScreen(self.hwnd(), &mut p);
        }
        Ok(UiaRect {
            left: f64::from(p.x),
            top: f64::from(p.y),
            width: f64::from((r.2 - r.0).max(0.0)),
            height: f64::from((r.3 - r.1).max(0.0)),
        })
    }

    fn GetEmbeddedFragmentRoots(&self) -> windows::core::Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> windows::core::Result<()> {
        self.post(Act::Focus)
    }

    fn FragmentRoot(&self) -> windows::core::Result<IRawElementProviderFragmentRoot> {
        Ok(self.sibling(None).into())
    }
}

#[allow(non_snake_case)]
impl IRawElementProviderFragmentRoot_Impl for Elem_Impl {
    fn ElementProviderFromPoint(
        &self,
        x: f64,
        y: f64,
    ) -> windows::core::Result<IRawElementProviderFragment> {
        let mut p = POINT {
            x: x as i32,
            y: y as i32,
        };
        // SAFETY: screen to client for the window (thread-safe).
        unsafe {
            let _ = ScreenToClient(self.hwnd(), &mut p);
        }
        let s = snapshot_for(self.hwnd);
        let (px, py) = (p.x as f32, p.y as f32);
        let hit = s.nodes.iter().find(|n| {
            px >= n.rect.left && px < n.rect.right && py >= n.rect.top && py < n.rect.bottom
        });
        Ok(provider(self.sibling(hit.map(|n| n.id))))
    }

    fn GetFocus(&self) -> windows::core::Result<IRawElementProviderFragment> {
        let s = snapshot_for(self.hwnd);
        match s.focus {
            Some(id) if s.nodes.iter().any(|n| n.id == id) => Ok(provider(self.sibling(Some(id)))),
            _ => Err(windows::core::Error::empty()),
        }
    }
}

#[allow(non_snake_case)]
impl IInvokeProvider_Impl for Elem_Impl {
    fn Invoke(&self) -> windows::core::Result<()> {
        self.post(Act::Activate)
    }
}

#[allow(non_snake_case)]
impl IToggleProvider_Impl for Elem_Impl {
    fn Toggle(&self) -> windows::core::Result<()> {
        self.post(Act::Activate)
    }

    fn ToggleState(&self) -> windows::core::Result<ToggleState> {
        let n = self.node(&snapshot_for(self.hwnd)).ok_or(E_FAIL)?;
        Ok(if n.toggled == Some(true) {
            ToggleState_On
        } else {
            ToggleState_Off
        })
    }
}

#[allow(non_snake_case)]
impl IExpandCollapseProvider_Impl for Elem_Impl {
    fn Expand(&self) -> windows::core::Result<()> {
        let n = self.node(&snapshot_for(self.hwnd)).ok_or(E_FAIL)?;
        if n.expanded == Some(false) {
            self.post(Act::Activate)?;
        }
        Ok(())
    }

    fn Collapse(&self) -> windows::core::Result<()> {
        let n = self.node(&snapshot_for(self.hwnd)).ok_or(E_FAIL)?;
        if n.expanded == Some(true) {
            self.post(Act::Activate)?;
        }
        Ok(())
    }

    fn ExpandCollapseState(&self) -> windows::core::Result<ExpandCollapseState> {
        let n = self.node(&snapshot_for(self.hwnd)).ok_or(E_FAIL)?;
        Ok(match n.expanded {
            Some(true) => ExpandCollapseState_Expanded,
            Some(false) => ExpandCollapseState_Collapsed,
            None => ExpandCollapseState_LeafNode,
        })
    }
}

#[allow(non_snake_case)]
impl ISelectionItemProvider_Impl for Elem_Impl {
    fn Select(&self) -> windows::core::Result<()> {
        let n = self.node(&snapshot_for(self.hwnd)).ok_or(E_FAIL)?;
        if !n.selected {
            self.post(Act::Activate)?;
        }
        Ok(())
    }

    fn AddToSelection(&self) -> windows::core::Result<()> {
        Err(E_INVALIDARG.into())
    }

    fn RemoveFromSelection(&self) -> windows::core::Result<()> {
        Err(E_INVALIDARG.into())
    }

    fn IsSelected(&self) -> windows::core::Result<BOOL> {
        let n = self.node(&snapshot_for(self.hwnd)).ok_or(E_FAIL)?;
        Ok(n.selected.into())
    }

    fn SelectionContainer(&self) -> windows::core::Result<IRawElementProviderSimple> {
        Ok(self.sibling(None).into())
    }
}

#[cfg(test)]
mod per_window_tests {
    use super::*;
    use crate::pages::Role;

    fn snap(name: &str, on: bool) -> Snapshot {
        Snapshot {
            nodes: vec![Node {
                id: 1,
                name: name.into(),
                role: Role::Toggle,
                rect: crate::fluent::rectf(0.0, 0.0, 0.0, 0.0),
                toggled: Some(on),
                expanded: None,
                selected: false,
            }],
            focus: None,
        }
    }

    #[test]
    fn two_windows_keep_their_own_trees() {
        let mut store = HashMap::new();
        assert!(publish_into(&mut store, 1, snap("camera", true)));
        assert!(publish_into(&mut store, 2, snap("audio", false)));
        assert_eq!(store[&1].nodes[0].name, "camera");
        assert_eq!(store[&2].nodes[0].name, "audio");
        // Publishing for window 1 does not change window 2.
        assert!(publish_into(&mut store, 1, snap("camera-2", true)));
        assert_eq!(store[&2].nodes[0].name, "audio");
    }

    #[test]
    fn publish_detects_when_focused_tile_toggles() {
        let mut store = HashMap::new();
        assert!(publish_into(&mut store, 1, snap("camera", false)));
        assert!(
            !publish_into(&mut store, 1, snap("camera", false)),
            "unchanged"
        );
        assert!(publish_into(&mut store, 1, snap("camera", true)), "toggled");
    }
}
