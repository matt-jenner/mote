use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ChangeHint {
    PathChanged(PathBuf),
    RescanRoot(PathBuf),
}

pub struct WatchService {
    root: PathBuf,
    pending: HashMap<ChangeHint, Instant>,
}

impl WatchService {
    pub const COALESCE_WINDOW: Duration = Duration::from_millis(250);

    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            pending: HashMap::new(),
        }
    }

    pub fn map_notify(result: notify::Result<notify::Event>, root: &Path) -> Vec<ChangeHint> {
        match result {
            Ok(event) if event.paths.is_empty() => {
                vec![ChangeHint::RescanRoot(root.to_path_buf())]
            }
            Ok(event) => event
                .paths
                .into_iter()
                .map(ChangeHint::PathChanged)
                .collect(),
            Err(_) => vec![ChangeHint::RescanRoot(root.to_path_buf())],
        }
    }

    pub fn record(&mut self, hint: ChangeHint, now: Instant) {
        if matches!(&hint, ChangeHint::RescanRoot(root) if root == &self.root) {
            self.pending.clear();
        }
        self.pending.insert(hint, now);
    }

    pub fn drain_ready(&mut self, now: Instant) -> Vec<ChangeHint> {
        let mut ready = self
            .pending
            .iter()
            .filter(|(_, recorded)| now.duration_since(**recorded) >= Self::COALESCE_WINDOW)
            .map(|(hint, _)| hint.clone())
            .collect::<Vec<_>>();
        for hint in &ready {
            self.pending.remove(hint);
        }
        ready.sort_by_key(|hint| match hint {
            ChangeHint::PathChanged(path) | ChangeHint::RescanRoot(path) => path.clone(),
        });
        ready
    }
}
