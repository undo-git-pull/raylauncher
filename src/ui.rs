//! AppKit views. The window and its rows are created once per window config and
//! reused; each keystroke only updates label text, icons, and the window height.

use std::cell::Cell;
use std::collections::HashMap;

use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSAppearance, NSApplication, NSBezierPath, NSImageResizingMode, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSColor, NSControl, NSControlTextEditingDelegate, NSEvent,
    NSFocusRingType, NSFont, NSImage, NSImageScaling, NSImageView, NSLineBreakMode, NSPanel, NSResponder, NSScreen,
    NSTextField, NSTextFieldDelegate, NSTextView, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowCollectionBehavior, NSWindowDelegate,
    NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSDistributedNotificationCenter, NSEdgeInsets, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::catalog::{Icon, Item};
use crate::config::Window;

define_class!(
    /// Borderless panels refuse key status by default; raylauncher needs it for typing.
    #[unsafe(super(NSPanel, NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RaylauncherPanel"]
    pub struct Panel;

    impl Panel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }
    }
);

define_class!(
    /// Top-left origin, so layout math reads top to bottom.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RaylauncherFlipView"]
    pub struct FlipView;

    impl FlipView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

pub struct RowIvars {
    index: Cell<usize>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RaylauncherRowView"]
    #[ivars = RowIvars]
    pub struct RowView;

    impl RowView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        // Claim clicks that land on the row's labels and icon.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, p: NSPoint) -> Option<Retained<NSView>> {
            let f = self.frame();
            let inside = p.x >= f.origin.x && p.x < f.origin.x + f.size.width
                && p.y >= f.origin.y && p.y < f.origin.y + f.size.height;
            inside.then(|| Retained::into_super(self.retain()))
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _e: &NSEvent) {
            crate::app::on_row_click(self.ivars().index.get());
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RaylauncherController"]
    pub struct Controller;

    impl Controller {
        #[unsafe(method(remoteToggle:))]
        fn remote_toggle(&self, _n: &NSNotification) {
            crate::app::on_remote_toggle();
        }
    }

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSControlTextEditingDelegate for Controller {
        #[unsafe(method(controlTextDidChange:))]
        fn text_changed(&self, _n: &NSNotification) {
            crate::app::on_query_changed();
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn do_command(&self, _c: &NSControl, _t: &NSTextView, sel: Sel) -> bool {
            crate::app::on_command(sel)
        }
    }

    unsafe impl NSTextFieldDelegate for Controller {}

    unsafe impl NSWindowDelegate for Controller {
        #[unsafe(method(windowDidResignKey:))]
        fn resign_key(&self, _n: &NSNotification) {
            crate::app::on_resign_key();
        }
    }
);

pub const TOGGLE_NOTIFICATION: &str = "local.raylauncher.toggle";

impl Controller {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this: Retained<Self> = unsafe { msg_send![super(Self::alloc(mtm).set_ivars(())), init] };
        // `raylauncher toggle` posts this notification; lets other tools and tests open the window.
        unsafe {
            NSDistributedNotificationCenter::defaultCenter().addObserver_selector_name_object(
                &this,
                objc2::sel!(remoteToggle:),
                Some(&ns(TOGGLE_NOTIFICATION)),
                None,
            );
        }
        this
    }
}

/// Ask a running raylauncher to show or hide its window.
pub fn post_toggle() {
    unsafe {
        NSDistributedNotificationCenter::defaultCenter().postNotificationName_object_userInfo_deliverImmediately(
            &ns(TOGGLE_NOTIFICATION),
            None,
            None,
            true,
        );
    }
}

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

#[derive(Clone, Copy)]
struct Metrics {
    width: f64,
    pad: f64,
    input_h: f64,
    row_h: f64,
    icon: f64,
    title: f64,
    sub: f64,
}

impl Metrics {
    fn new(w: &Window) -> Metrics {
        let f = w.font_size;
        let row_h = (f * 2.3).round();
        Metrics {
            width: w.width,
            pad: 10.0,
            input_h: (f * 2.1).round(),
            row_h,
            icon: (row_h * 0.7).round(),
            title: (f * 0.72).round(),
            sub: (f * 0.52).round().max(10.0),
        }
    }
    fn height(&self, rows: usize) -> f64 {
        let list = if rows > 0 { 4.0 + rows as f64 * self.row_h } else { 0.0 };
        self.pad * 2.0 + self.input_h + list
    }
}

struct Row {
    view: Retained<RowView>,
    icon: Retained<NSImageView>,
    title: Retained<NSTextField>,
    sub: Retained<NSTextField>,
}

pub struct Ui {
    pub panel: Retained<Panel>,
    pub field: Retained<NSTextField>,
    rows: Vec<Row>,
    m: Metrics,
    top_offset: f64,
    show_icons: bool,
    top: f64,
    icons: HashMap<String, Retained<NSImage>>,
}

fn label(mtm: MainThreadMarker, size: f64) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&ns(""), mtm);
    l.setFont(Some(&NSFont::systemFontOfSize(size)));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}

impl Ui {
    pub fn build(mtm: MainThreadMarker, w: &Window, controller: &Controller) -> Ui {
        let m = Metrics::new(w);
        let h = m.height(0);
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let panel: Retained<Panel> = unsafe {
            msg_send![super(Panel::alloc(mtm).set_ivars(())),
                initWithContentRect: rect(0.0, 0.0, m.width, h),
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false]
        };
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setLevel(101); // NSPopUpMenuWindowLevel: above normal and floating windows
        panel.setHidesOnDeactivate(false);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Transient,
        );
        panel.setDelegate(Some(ProtocolObject::from_ref(controller)));
        let appearance = match w.theme.as_str() {
            "dark" => NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua }),
            "light" => NSAppearance::appearanceNamed(unsafe { NSAppearanceNameAqua }),
            _ => None,
        };
        panel.setAppearance(appearance.as_deref());

        let bg = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), rect(0.0, 0.0, m.width, h));
        bg.setMaterial(NSVisualEffectMaterial::Popover);
        bg.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        bg.setState(NSVisualEffectState::Active);
        // A layer corner radius does not clip behind-window blur; the window server fills
        // the corners. A stretchable mask image is the supported way to round it.
        bg.setMaskImage(Some(&rounded_mask(CORNER_RADIUS)));
        let bg_view: &NSView = &bg;
        panel.setContentView(Some(bg_view));

        let root: Retained<FlipView> = unsafe {
            msg_send![super(FlipView::alloc(mtm).set_ivars(())), initWithFrame: rect(0.0, 0.0, m.width, h)]
        };
        root.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        bg.addSubview(&root);

        // Search glyph + query field.
        let glyph_size = (w.font_size * 0.95).round();
        let glyph = NSImageView::initWithFrame(
            NSImageView::alloc(mtm),
            rect(m.pad + 8.0, m.pad + (m.input_h - glyph_size) / 2.0, glyph_size, glyph_size),
        );
        glyph.setImage(symbol("magnifyingglass").as_deref());
        glyph.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        glyph.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        root.addSubview(&glyph);

        let field_h = (w.font_size * 1.35).round();
        let fx = m.pad + 8.0 + glyph_size + 10.0;
        let field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            rect(fx, m.pad + (m.input_h - field_h) / 2.0, m.width - fx - m.pad - 8.0, field_h),
        );
        field.setBezeled(false);
        field.setDrawsBackground(false);
        field.setFocusRingType(NSFocusRingType::None);
        field.setFont(Some(&NSFont::systemFontOfSize(w.font_size)));
        field.setPlaceholderString(Some(&ns(&w.placeholder)));
        if let Some(cell) = field.cell() {
            cell.setScrollable(true);
            cell.setWraps(false);
        }
        // SAFETY: the controller lives in State for the life of the process (delegates are weak).
        unsafe { field.setDelegate(Some(ProtocolObject::from_ref(controller))) };
        root.addSubview(&field);

        let mut rows = Vec::with_capacity(w.max_results);
        let row_w = m.width - 2.0 * m.pad;
        let text_x = if w.show_icons { 10.0 + m.icon + 10.0 } else { 14.0 };
        for i in 0..w.max_results {
            let y = m.pad + m.input_h + 4.0 + i as f64 * m.row_h;
            let view: Retained<RowView> = unsafe {
                msg_send![super(RowView::alloc(mtm).set_ivars(RowIvars { index: Cell::new(i) })),
                    initWithFrame: rect(m.pad, y, row_w, m.row_h)]
            };
            view.setWantsLayer(true);
            if let Some(layer) = view.layer() {
                layer.setCornerRadius(8.0);
            }
            let icon = NSImageView::initWithFrame(
                NSImageView::alloc(mtm),
                rect(10.0, (m.row_h - m.icon) / 2.0, m.icon, m.icon),
            );
            icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            icon.setHidden(!w.show_icons);
            let title = label(mtm, m.title);
            let sub = label(mtm, m.sub);
            sub.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
            let title_h = (m.title * 1.3).round();
            let sub_h = (m.sub * 1.35).round();
            let top = ((m.row_h - title_h - sub_h) / 2.0).round();
            let tw = row_w - text_x - 10.0;
            title.setFrame(rect(text_x, top, tw, title_h));
            sub.setFrame(rect(text_x, top + title_h, tw, sub_h));
            view.addSubview(&icon);
            view.addSubview(&title);
            view.addSubview(&sub);
            view.setHidden(true);
            root.addSubview(&view);
            rows.push(Row { view, icon, title, sub });
        }

        Ui { panel, field, rows, m, top_offset: w.top_offset, show_icons: w.show_icons, top: 0.0, icons: HashMap::new() }
    }

    pub fn is_visible(&self) -> bool {
        self.panel.isVisible()
    }

    pub fn query(&self) -> String {
        self.field.stringValue().to_string()
    }

    /// Show on the screen under the mouse pointer, with an empty query.
    pub fn show(&mut self, mtm: MainThreadMarker) {
        let mouse = NSEvent::mouseLocation();
        let screens = NSScreen::screens(mtm);
        let screen = screens
            .iter()
            .find(|s| {
                let f = s.frame();
                mouse.x >= f.origin.x
                    && mouse.x < f.origin.x + f.size.width
                    && mouse.y >= f.origin.y
                    && mouse.y < f.origin.y + f.size.height
            })
            .or_else(|| screens.firstObject());
        if let Some(s) = screen {
            let vf = s.visibleFrame();
            let full = s.frame();
            self.top = vf.origin.y + vf.size.height - full.size.height * self.top_offset;
            let x = vf.origin.x + (vf.size.width - self.m.width) / 2.0;
            let h = self.panel.frame().size.height;
            self.panel.setFrame_display(rect(x.round(), (self.top - h).round(), self.m.width, h), false);
        }
        self.field.setStringValue(&ns(""));
        // Activate so keystrokes reach the field even when another app owns keyboard focus.
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        self.panel.makeKeyAndOrderFront(None);
        self.panel.invalidateShadow(); // shadow follows the rounded mask, not the square frame
        self.panel.makeFirstResponder(Some(&self.field));
    }

    /// Hide the window and, if we still own focus, hand it back to the previous app.
    pub fn hide(&self, mtm: MainThreadMarker) {
        self.panel.orderOut(None);
        let app = NSApplication::sharedApplication(mtm);
        if app.isActive() {
            app.hide(None);
        }
    }

    pub fn close(&self) {
        self.panel.close();
    }

    pub fn max_rows(&self) -> usize {
        self.rows.len()
    }

    /// Fill rows with `items`, highlight `selected`, and resize the window to fit.
    /// `note` replaces the subtitle of the selected row (used for "press Enter again").
    pub fn render(&mut self, items: &[Item], selected: usize, note: Option<&str>) {
        let n = items.len().min(self.rows.len());
        if self.icons.len() > 128 {
            self.icons.clear();
        }
        let accent = NSColor::controlAccentColor();
        for i in 0..self.rows.len() {
            let row = &self.rows[i];
            if i >= n {
                row.view.setHidden(true);
                continue;
            }
            let it = &items[i];
            let sel = i == selected;
            row.title.setStringValue(&ns(&it.title));
            let sub = if sel { note.unwrap_or(&it.subtitle) } else { &it.subtitle };
            row.sub.setStringValue(&ns(sub));
            let (fg, fg2) = if sel {
                (NSColor::whiteColor(), NSColor::whiteColor().colorWithAlphaComponent(0.8))
            } else {
                (NSColor::labelColor(), NSColor::secondaryLabelColor())
            };
            row.title.setTextColor(Some(&fg));
            row.sub.setTextColor(Some(&fg2));
            if let Some(layer) = row.view.layer() {
                let c = sel.then(|| accent.CGColor());
                layer.setBackgroundColor(c.as_deref());
            }
            if self.show_icons {
                let img = icon_for(&mut self.icons, &it.icon);
                row.icon.setImage(img.as_deref());
                row.icon.setContentTintColor(Some(&fg2));
            }
            row.view.setHidden(false);
        }
        let h = self.m.height(n);
        let f = self.panel.frame();
        if (f.size.height - h).abs() > 0.5 {
            let top = if self.top > 0.0 { self.top } else { f.origin.y + f.size.height };
            self.panel.setFrame_display(rect(f.origin.x, top - h, f.size.width, h), true);
            self.panel.invalidateShadow();
        }
    }
}

const CORNER_RADIUS: f64 = 14.0;

/// A rounded-rect alpha mask that stretches to any size: only the corners are drawn,
/// cap insets keep them fixed, and the middle scales.
fn rounded_mask(r: f64) -> Retained<NSImage> {
    let side = r * 2.0 + 1.0;
    let draw = block2::RcBlock::new(move |bounds: NSRect| -> Bool {
        NSColor::blackColor().setFill();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, r, r).fill();
        Bool::YES
    });
    let img = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(side, side), false, &draw);
    img.setCapInsets(NSEdgeInsets { top: r, left: r, bottom: r, right: r });
    // NSImageResizingModeStretch is 1 where TARGET_ABI_USES_IOS_VALUES (Apple silicon) and 0 on
    // Intel (AppKit/NSImage.h). objc2-app-kit 0.3.2's `Stretch` constant has these swapped.
    let stretch = if cfg!(target_arch = "x86_64") { 0 } else { 1 };
    img.setResizingMode(NSImageResizingMode(stretch));
    img
}

fn symbol(name: &str) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns(name), None)
}

fn icon_for(cache: &mut HashMap<String, Retained<NSImage>>, icon: &Icon) -> Option<Retained<NSImage>> {
    let key = match icon {
        Icon::None => return None,
        Icon::File(p) => p.to_string_lossy().into_owned(),
        Icon::Symbol(s) => format!("sf:{s}"),
    };
    if let Some(img) = cache.get(&key) {
        return Some(img.clone());
    }
    let img = match icon {
        Icon::File(p) => Some(NSWorkspace::sharedWorkspace().iconForFile(&ns(&p.to_string_lossy()))),
        Icon::Symbol(s) => symbol(s),
        Icon::None => None,
    }?;
    cache.insert(key, img.clone());
    Some(img)
}
