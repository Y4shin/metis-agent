//! Tiny result recorder for the spike host.

pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub note: String,
}

#[derive(Default)]
pub struct Checks {
    items: Vec<Check>,
}

impl Checks {
    pub fn new() -> Self {
        Checks { items: Vec::new() }
    }

    pub fn check(&mut self, ok: bool, name: &'static str, note: impl Into<String>) -> bool {
        let item = Check { name, ok, note: note.into() };
        // Printed as it happens: if the spike ever hangs, the last line names
        // the step that did not finish.
        let mark = if item.ok { "PASS" } else { "FAIL" };
        println!("{mark}  {:<48} {}", item.name, item.note);
        self.items.push(item);
        ok
    }

    pub fn all_ok(&self) -> bool {
        self.items.iter().all(|item| item.ok)
    }

    pub fn failed(&self) -> usize {
        self.items.iter().filter(|item| !item.ok).count()
    }

    pub fn report(&self) {
        println!("---\n{} checks, {} failed", self.items.len(), self.failed());
    }
}
