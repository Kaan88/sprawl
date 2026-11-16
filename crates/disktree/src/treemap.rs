//! Painting the treemap into a bitmap and handling the mouse over it.

use windows_core::Interface;

use crate::bindings::Microsoft::UI::Xaml::Controls::{Image, ScrollViewer};
use crate::bindings::Microsoft::UI::Xaml::Media::ImageSource;
use crate::bindings::Microsoft::UI::Xaml::FrameworkElement;
use crate::state::{find, text, State, View, MAP_INSET, ROW_H};
use crate::{app, paint, win32};

impl State {
    // ------------------------------------------------------------ treemap

    pub(crate) fn scale(&self) -> f64 {
        win32::dpi(self.hwnd) as f64 / 96.0
    }

    pub(crate) fn render_map(&mut self) {
        let Some(t) = &self.tree else { return };
        let Ok(img) = find::<FrameworkElement>(&self.root, "MapImage") else { return };
        // Measure the frame, not the Image: an Image without a picture yet has no size.
        let Ok(frame) = find::<FrameworkElement>(&self.root, "MapBorder") else { return };
        let s = self.scale();
        let inner = |v: f64| ((v - 2.0 * MAP_INSET) * s).round().max(0.0) as usize;
        let w = inner(frame.ActualWidth().unwrap_or(0.0));
        let h = inner(frame.ActualHeight().unwrap_or(0.0));
        if w < 8 || h < 8 {
            return;
        }
        let lay = paint::layout(t, self.map_root, w as f64, h as f64);
        let mut px = vec![0u8; w * h * 4];
        let hx = self.ext_selected.and_then(|k| self.exts.get(k)).map(|e| e.ext.as_str());
        paint::paint(t, &lay, &self.colours, hx, self.selected, w, h, &mut px);
        let shown = || -> windows_core::Result<()> {
            let bmp = app::bitmap(w, h, &px)?;
            img.cast::<Image>()?.SetSource(&bmp.cast::<ImageSource>()?)?;
            Ok(())
        };
        if let Err(e) = shown() {
            eprintln!("treemap: {e:?}");
        }
        self.layout = Some(lay);
        self.map_px = (w, h);
        self.map_pixels = px;
        text(&self.root, "MapPath", &format!("[{}]", t.path_of(self.map_root).display()));
    }

    pub(crate) fn map_node_at(&self, x: f64, y: f64) -> Option<usize> {
        let s = self.scale();
        self.layout.as_ref()?.hit(x * s, y * s)
    }

    pub(crate) fn map_hover(&mut self, x: f64, y: f64) {
        let n = self.map_node_at(x, y);
        if n == self.hover {
            return;
        }
        self.hover = n;
        if let (Some(t), Some(n)) = (&self.tree, n) {
            text(&self.root, "MapPath", &format!("{}   ({})", t.path_of(n).display(), self.size(t.nodes[n].size)));
        }
    }

    pub(crate) fn map_click(&mut self, x: f64, y: f64) {
        let Some(n) = self.map_node_at(x, y) else { return };
        self.reveal(n);
    }

    /// Select a node and open every folder above it, then scroll it into view.
    pub(crate) fn reveal(&mut self, n: usize) {
        let Some(t) = &self.tree else { return };
        let mut p = t.nodes[n].parent;
        while let Some(i) = p {
            self.expanded.insert(i);
            p = t.nodes[i].parent;
        }
        self.selected = Some(n);
        if self.view != View::Tree {
            self.set_view(View::Tree);
        }
        self.refresh_rows();
        self.render_map();
        self.show_selection();
        if let Some(k) = self.rows.iter().position(|r| r.0 == n) {
            if let Ok(sv) = find::<ScrollViewer>(&self.root, "TreeScroll") {
                let _ = sv.ChangeView(None, Some((k as f64 * ROW_H - 120.0).max(0.0)), None);
            }
        }
    }
}
