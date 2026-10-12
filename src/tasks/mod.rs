use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use uuid::Uuid;

const MAX_TASK_HISTORY: usize = 20;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Fetch,
    Speedtest,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskPhase {
    Queued,
    Downloading,
    Parsing,
    Writing,
    TcpPrefilter,
    GoogleVerify,
    Updating,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceTaskSnapshot {
    pub source_id: String,
    pub source_name: String,
    pub phase: TaskPhase,
    pub node_count: usize,
    pub download_ms: Option<u64>,
    pub parse_ms: Option<u64>,
    pub write_ms: Option<u64>,
    pub speedtest_ms: Option<u64>,
    pub update_ms: Option<u64>,
    pub elapsed_ms: u64,
    pub message: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskSnapshot {
    pub id: String,
    pub kind: TaskKind,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub total_sources: usize,
    pub completed_sources: usize,
    pub successful_sources: usize,
    pub failed_sources: usize,
    pub elapsed_ms: u64,
    pub sources: Vec<SourceTaskSnapshot>,
}

#[derive(Clone, Default)]
pub struct TaskTracker {
    inner: Arc<Mutex<Vec<TaskState>>>,
}

#[derive(Clone)]
struct SourceState {
    snapshot: SourceTaskSnapshot,
    started: Instant,
}

struct TaskState {
    snapshot: TaskSnapshot,
    started: Instant,
    sources: BTreeMap<String, SourceState>,
}

impl TaskTracker {
    pub fn start(
        &self,
        kind: TaskKind,
        sources: impl IntoIterator<Item = (String, String)>,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let started = Instant::now();
        let mut states = BTreeMap::new();
        for (source_id, source_name) in sources {
            states.insert(
                source_id.clone(),
                SourceState {
                    snapshot: SourceTaskSnapshot {
                        source_id,
                        source_name,
                        phase: TaskPhase::Queued,
                        node_count: 0,
                        download_ms: None,
                        parse_ms: None,
                        write_ms: None,
                        speedtest_ms: None,
                        update_ms: None,
                        elapsed_ms: 0,
                        message: None,
                        error: None,
                    },
                    started,
                },
            );
        }
        let empty = states.is_empty();
        let snapshot = TaskSnapshot {
            id: id.clone(),
            kind,
            started_at: Utc::now(),
            finished_at: empty.then(Utc::now),
            total_sources: states.len(),
            completed_sources: 0,
            successful_sources: 0,
            failed_sources: 0,
            elapsed_ms: 0,
            sources: Vec::new(),
        };
        let mut tasks = self.inner.lock().expect("task tracker lock poisoned");
        tasks.insert(
            0,
            TaskState {
                snapshot,
                started,
                sources: states,
            },
        );
        tasks.truncate(MAX_TASK_HISTORY);
        id
    }

    pub fn phase(&self, task_id: &str, source_id: &str, phase: TaskPhase) {
        self.update(task_id, source_id, |source| source.phase = phase);
    }

    pub fn timings(
        &self,
        task_id: &str,
        source_id: &str,
        download_ms: Option<u64>,
        parse_ms: Option<u64>,
        write_ms: Option<u64>,
        speedtest_ms: Option<u64>,
        update_ms: Option<u64>,
    ) {
        self.update(task_id, source_id, |source| {
            if download_ms.is_some() {
                source.download_ms = download_ms;
            }
            if parse_ms.is_some() {
                source.parse_ms = parse_ms;
            }
            if write_ms.is_some() {
                source.write_ms = write_ms;
            }
            if speedtest_ms.is_some() {
                source.speedtest_ms = speedtest_ms;
            }
            if update_ms.is_some() {
                source.update_ms = update_ms;
            }
        });
    }

    pub fn nodes(&self, task_id: &str, source_id: &str, node_count: usize) {
        self.update(task_id, source_id, |source| source.node_count = node_count);
    }

    pub fn complete(&self, task_id: &str, source_id: &str, message: impl Into<String>) {
        self.finish_source(task_id, source_id, true, message.into());
    }

    pub fn fail(&self, task_id: &str, source_id: &str, error: impl Into<String>) {
        self.finish_source(task_id, source_id, false, error.into());
    }

    pub fn snapshots(&self, limit: usize) -> Vec<TaskSnapshot> {
        let tasks = self.inner.lock().expect("task tracker lock poisoned");
        tasks
            .iter()
            .take(limit.clamp(1, MAX_TASK_HISTORY))
            .map(snapshot)
            .collect()
    }

    pub fn is_source_active(&self, source_id: &str) -> bool {
        let tasks = self.inner.lock().expect("task tracker lock poisoned");
        tasks.iter().any(|task| {
            task.sources.get(source_id).is_some_and(|source| {
                !matches!(
                    source.snapshot.phase,
                    TaskPhase::Completed | TaskPhase::Failed
                )
            })
        })
    }

    pub fn has_active_fetch(&self) -> bool {
        let tasks = self.inner.lock().expect("task tracker lock poisoned");
        tasks.iter().any(|task| {
            task.snapshot.kind == TaskKind::Fetch && task.snapshot.finished_at.is_none()
        })
    }

    fn update(&self, task_id: &str, source_id: &str, update: impl FnOnce(&mut SourceTaskSnapshot)) {
        let mut tasks = self.inner.lock().expect("task tracker lock poisoned");
        if let Some(task) = tasks.iter_mut().find(|task| task.snapshot.id == task_id) {
            if let Some(source) = task.sources.get_mut(source_id) {
                update(&mut source.snapshot);
                source.snapshot.elapsed_ms = source.started.elapsed().as_millis() as u64;
            }
        }
    }

    fn finish_source(&self, task_id: &str, source_id: &str, ok: bool, text: String) {
        let mut tasks = self.inner.lock().expect("task tracker lock poisoned");
        let Some(task) = tasks.iter_mut().find(|task| task.snapshot.id == task_id) else {
            return;
        };
        let Some(source) = task.sources.get_mut(source_id) else {
            return;
        };
        if matches!(
            source.snapshot.phase,
            TaskPhase::Completed | TaskPhase::Failed
        ) {
            return;
        }
        source.snapshot.phase = if ok {
            TaskPhase::Completed
        } else {
            TaskPhase::Failed
        };
        source.snapshot.elapsed_ms = source.started.elapsed().as_millis() as u64;
        if ok {
            source.snapshot.message = Some(text);
            task.snapshot.successful_sources += 1;
        } else {
            source.snapshot.error = Some(text);
            task.snapshot.failed_sources += 1;
        }
        task.snapshot.completed_sources += 1;
        let all_sources_finished = task.sources.values().all(|state| {
            matches!(
                state.snapshot.phase,
                TaskPhase::Completed | TaskPhase::Failed
            )
        });
        if task.snapshot.completed_sources == task.snapshot.total_sources || all_sources_finished {
            task.snapshot.elapsed_ms = task.started.elapsed().as_millis() as u64;
            task.snapshot.finished_at = Some(Utc::now());
        }
    }
}

fn snapshot(task: &TaskState) -> TaskSnapshot {
    let mut value = task.snapshot.clone();
    if value.finished_at.is_none() {
        value.elapsed_ms = task.started.elapsed().as_millis() as u64;
    }
    value.sources = task
        .sources
        .values()
        .map(|state| {
            let mut source = state.snapshot.clone();
            if !matches!(source.phase, TaskPhase::Completed | TaskPhase::Failed) {
                source.elapsed_ms = state.started.elapsed().as_millis() as u64;
            }
            source
        })
        .collect();
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;
    #[test]
    fn tracks_success_and_failure() {
        let tracker = TaskTracker::default();
        let id = tracker.start(
            TaskKind::Fetch,
            [("a".into(), "A".into()), ("b".into(), "B".into())],
        );
        tracker.phase(&id, "a", TaskPhase::Downloading);
        tracker.complete(&id, "a", "ok");
        tracker.fail(&id, "b", "broken");
        let task = tracker.snapshots(1).pop().unwrap();
        assert_eq!(task.completed_sources, 2);
        assert_eq!(task.successful_sources, 1);
        assert_eq!(task.failed_sources, 1);
    }

    #[test]
    fn finished_task_and_source_elapsed_times_stop_growing() {
        let tracker = TaskTracker::default();
        let id = tracker.start(TaskKind::Fetch, [("a".into(), "A".into())]);
        tracker.fail(&id, "a", "broken");
        let first = tracker.snapshots(1).pop().unwrap();
        thread::sleep(Duration::from_millis(20));
        let second = tracker.snapshots(1).pop().unwrap();

        assert_eq!(first.elapsed_ms, second.elapsed_ms);
        assert_eq!(first.sources[0].elapsed_ms, second.sources[0].elapsed_ms);
        assert!(second.finished_at.is_some());
    }

    #[test]
    fn active_source_tracking_clears_after_failure() {
        let tracker = TaskTracker::default();
        let id = tracker.start(TaskKind::Fetch, [("a".into(), "A".into())]);
        assert!(tracker.is_source_active("a"));
        assert!(tracker.has_active_fetch());

        tracker.fail(&id, "a", "broken");

        assert!(!tracker.is_source_active("a"));
        assert!(!tracker.has_active_fetch());
    }
}
