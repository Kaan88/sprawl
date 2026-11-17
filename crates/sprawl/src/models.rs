//! The Model dropdown: the provider's list of models, fetched on another thread. While it loads,
//! or when there is no list (no key yet, a server that's down), the model is typed
//! in a text box instead.

use std::sync::mpsc::{channel, Receiver};

use sprawl_advisor::config::Provider;

type Answer = Result<Vec<String>, String>;

#[derive(Default)]
pub(crate) struct ModelFetch {
    /// The provider and address the list is for.
    key: Option<(Provider, String)>,
    rx: Option<Receiver<Answer>>,
    result: Option<Answer>,
}

impl ModelFetch {
    /// Fetch the list for `p` at `url`, unless it is already fetched or on its way.
    pub(crate) fn want(&mut self, p: Provider, url: &str) {
        let k = (p, url.trim().to_string());
        if self.key.as_ref() == Some(&k) {
            return;
        }
        self.key = Some(k.clone());
        self.result = None;
        if p == Provider::Off {
            self.rx = None;
            return;
        }
        let (tx, rx) = channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(|| sprawl_advisor::llm::models::list(k.0, &k.1)).unwrap_or_else(|_| Err("the list couldn't be read".into()));
            let _ = tx.send(r);
        });
    }

    /// Forget the list, so the next `want` fetches again (a key was just saved).
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Each frame: true when the list just arrived.
    pub(crate) fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok(r) => {
                self.result = Some(r);
                self.rx = None;
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.result = Some(Err("the list couldn't be read".into()));
                self.rx = None;
                true
            }
        }
    }

    /// The dropdown's entries: the list, with `current` first if the list doesn't have it.
    /// `None`: no list, so a text box.
    pub(crate) fn choices(&self, current: &str) -> Option<Vec<String>> {
        let Some(Ok(list)) = &self.result else { return None };
        let mut v = list.clone();
        let c = current.trim();
        if !c.is_empty() && !v.iter().any(|m| m == c) {
            v.insert(0, c.to_string());
        }
        Some(v)
    }

    /// A line under the field: loading, or why there's no list.
    pub(crate) fn note(&self) -> Option<String> {
        if self.rx.is_some() {
            return Some("Loading the list of models...".into());
        }
        match &self.result {
            Some(Err(e)) => Some(format!("No list of models ({e}), so type the name.")),
            _ => None,
        }
    }
}

/// A ComboBox named `name` with `list`, `current` selected.
pub(crate) fn combo(name: &str, list: &[String], current: &str, width: u32) -> String {
    let sel = list.iter().position(|m| m == current.trim()).unwrap_or(0);
    let items: String = list.iter().map(|m| format!(r##"<ComboBoxItem Content="{}"/>"##, crate::ui::esc(m))).collect();
    format!(r##"<ComboBox x:Name="{name}" Width="{width}" MaxDropDownHeight="360" SelectedIndex="{sel}">{items}</ComboBox>"##)
}
