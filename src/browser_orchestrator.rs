//! 浏览器分析任务的 Rust 编排器核心。负责前台/后台任务入队、任务去重、任务调度、前台请求状态与进度统计。
use std::collections::{HashMap, VecDeque};

/// 任务优先级。
/// 前台请求默认高于后台请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisPriority {
    /// 前台分析。
    Foreground,
    /// 后台扫描。
    Background,
}

/// 队列状态。
/// 反映当前调度器整体是否在运行、空闲、暂停或完成。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueStatus {
    /// 队列存在任务，尚未启动。
    Idle,
    /// 当前有活动任务。
    Running,
    /// 外部暂停，未允许启动新任务。
    Paused,
    /// 队列与运行任务均已处理完。
    Completed,
}

/// 前台 selection 请求状态。
/// 表示某次前台请求当前所处的生命周期。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForegroundRequestState {
    /// 等待产物。
    Pending,
    /// 已完成。
    Completed,
    /// 被后续 generation 覆盖，丢弃。
    Stale,
    /// 命中已缓存产物。
    CacheHit,
}

/// 产物作用域。
/// 同一个 artifact key 在不同用途下不共享队列边界。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisArtifactScope {
    /// 前台请求产物。
    Foreground,
    /// 后台扫描产物。
    Background,
}

/// 产物查找键。
/// 该结构体用于唯一定位一次分析产物，参与去重和缓存命中。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AnalysisArtifactKey {
    /// 项目名（可选），用于多项目上下文隔离。
    pub project_name: Option<String>,
    /// 元数据源路径。
    pub source_path: String,
    /// 文件标识（可选）。
    pub file_id: Option<String>,
    /// 文件版本号（可选）。
    pub revision: Option<String>,
    /// 激活组件 ID（可选）。
    pub active_component_id: Option<String>,
    /// 当前选中组件列表。
    pub selected_component_ids: Vec<String>,
    /// scope 用于区分同 key 的前后台职责。
    pub scope: AnalysisArtifactScope,
}

impl AnalysisArtifactKey {
    /// 构造前台请求的产物 key。
    pub fn foreground(
        project_name: Option<String>,
        source_path: impl Into<String>,
        file_id: Option<String>,
        revision: Option<String>,
        active_component_id: Option<String>,
        selected_component_ids: Vec<String>,
    ) -> Self {
        Self {
            project_name,
            source_path: source_path.into(),
            file_id,
            revision,
            active_component_id,
            selected_component_ids,
            scope: AnalysisArtifactScope::Foreground,
        }
    }

    /// 构造后台请求的产物 key。
    pub fn background(
        project_name: Option<String>,
        source_path: impl Into<String>,
        file_id: Option<String>,
        revision: Option<String>,
    ) -> Self {
        Self {
            project_name,
            source_path: source_path.into(),
            file_id,
            revision,
            active_component_id: None,
            selected_component_ids: Vec::new(),
            scope: AnalysisArtifactScope::Background,
        }
    }
}

/// 分析产物。
/// completion_tick 与 generation 共同用于后续的稳定缓存与结果可追踪。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisArtifact {
    /// 产物对应 key。
    pub key: AnalysisArtifactKey,
    /// 产物 sequence，默认与 generation 对齐。
    pub sequence: u64,
    /// 任务完成 tick。
    pub ready_tick: u64,
}

/// 背景或前台扫描任务。
/// 每个 task_id 只承载一次执行。
#[derive(Debug, Clone)]
pub struct BackgroundScanTask {
    /// 绑定的 artifact key。
    pub artifact_key: AnalysisArtifactKey,
    /// 优先级，前台先于后台调度。
    pub priority: AnalysisPriority,
    /// generation，控制 stale 和覆盖逻辑。
    pub generation: u64,
    /// 任务耗时（tick）。
    pub processing_ticks: u64,
}

/// 前台 selection 请求对象。
/// 与前台请求一一对应，用于状态回写。
#[derive(Debug, Clone)]
pub struct ForegroundSelectionRequest {
    /// 请求 ID。
    pub request_id: u64,
    /// 绑定到的 key。
    pub artifact_key: AnalysisArtifactKey,
    /// 本次请求 generation。
    pub generation: u64,
    /// 关联 task id。
    pub linked_task_id: u64,
    /// 当前请求状态。
    pub state: ForegroundRequestState,
    /// 若已完成/命中缓存返回 artifact。
    pub artifact: Option<AnalysisArtifact>,
}

/// 前台请求入队结果。
/// 入队结果反映是否创建任务、是否复用、是否命中缓存。
pub struct ForegroundSelectionEnqueueResult {
    /// 该次返回的 request id。
    pub request_id: u64,
    /// 命中的 task id，cache hit 时为空。
    pub task_id: Option<u64>,
    /// 去重合并对象（同 key 同代重复入队时）。
    pub merged_with: Option<u64>,
    /// 是否命中缓存。
    pub cache_hit: bool,
}

/// 执行进度。
/// 供观测层展示队列与并发执行指标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundProgress {
    /// 队列状态。
    pub status: QueueStatus,
    /// 已完成任务数。
    pub processed: usize,
    /// 总计任务量（processed + active + queued）。
    pub total: usize,
    /// 当前运行中的任务数。
    pub active: usize,
    /// 当前队列中的任务数。
    pub queued: usize,
    /// 同时允许的最大并发。
    pub max_concurrency: usize,
    /// 本次 tick 限制的拉起上限。
    pub limit: usize,
    /// 最小任务启动间隔 tick。
    pub min_interval_ticks: u64,
}

#[derive(Debug, Clone)]
struct RunningTask {
    task: BackgroundScanTask,
    complete_at: u64,
}

/// 已启动或已完成任务的结构化描述。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchestratorTaskDescriptor {
    /// 任务 ID。
    pub task_id: u64,
    /// 元数据源路径。
    pub source_path: String,
    /// 项目名（可选）。
    pub project_name: Option<String>,
    /// 文件标识（可选）。
    pub file_id: Option<String>,
    /// 文件版本（可选）。
    pub revision: Option<String>,
    /// 产物作用域。
    pub scope: AnalysisArtifactScope,
    /// generation。
    pub generation: u64,
    /// 激活组件 ID（前台可见）。
    pub active_component_id: Option<String>,
    /// 当前选中组件列表（前台可见）。
    pub selected_component_ids: Vec<String>,
}

/// 一次 tick 的执行结果。
/// 测试和状态检查可基于该结果判断本轮行为。
#[derive(Debug, Clone)]
pub struct OrchestratorTickResult {
    /// 本轮新增启动的任务 id。
    pub started_task_ids: Vec<u64>,
    /// 本轮新增启动任务描述。
    pub started_task_descriptors: Vec<OrchestratorTaskDescriptor>,
    /// 本轮完成的任务 id。
    pub completed_task_ids: Vec<u64>,
    /// 本轮完成任务描述。
    pub completed_task_descriptors: Vec<OrchestratorTaskDescriptor>,
    /// 本轮完成的前台请求 id。
    pub completed_request_ids: Vec<u64>,
    /// 本轮进度快照。
    pub background_progress: BackgroundProgress,
}

/// 浏览器分析 orchestrator 核心状态与队列调度器。
#[derive(Debug)]
pub struct BrowserAnalysisOrchestrator {
    max_concurrency: usize,
    min_interval_ticks: u64,
    paused: bool,
    now_tick: u64,
    next_start_tick: u64,
    next_request_id: u64,
    next_task_id: u64,

    tasks: HashMap<u64, BackgroundScanTask>,
    foreground_queue: VecDeque<u64>,
    background_queue: VecDeque<u64>,
    running: HashMap<u64, RunningTask>,

    /// key 到 task_id 的映射，避免同一 artifact_key 重复入队。
    task_by_key: HashMap<AnalysisArtifactKey, u64>,

    artifact_cache: HashMap<AnalysisArtifactKey, AnalysisArtifact>,
    requests: HashMap<u64, ForegroundSelectionRequest>,

    /// 每个 key 当前有效 request。
    latest_request_by_key: HashMap<AnalysisArtifactKey, u64>,
    latest_generation_by_key: HashMap<AnalysisArtifactKey, u64>,
    request_waiting_by_key: HashMap<AnalysisArtifactKey, u64>,

    processed_count: usize,
}

impl BrowserAnalysisOrchestrator {
    /// 创建新的编排器实例。
    /// `max_concurrency` 最低为 1，避免 0 并发导致任务永远不启动。
    pub fn new(max_concurrency: usize, min_interval_ticks: u64) -> Self {
        let max_concurrency = max_concurrency.max(1);

        Self {
            max_concurrency,
            min_interval_ticks,
            paused: false,
            now_tick: 0,
            next_start_tick: 0,
            next_request_id: 1,
            next_task_id: 1,
            tasks: HashMap::new(),
            foreground_queue: VecDeque::new(),
            background_queue: VecDeque::new(),
            running: HashMap::new(),
            task_by_key: HashMap::new(),
            artifact_cache: HashMap::new(),
            requests: HashMap::new(),
            latest_request_by_key: HashMap::new(),
            latest_generation_by_key: HashMap::new(),
            request_waiting_by_key: HashMap::new(),
            processed_count: 0,
        }
    }

    /// 更新最大并发数，最终下限仍为 1。
    pub fn set_max_concurrency(&mut self, max_concurrency: usize) {
        self.max_concurrency = max_concurrency.max(1);
    }

    /// 更新最小启动间隔 tick，0 表示无间隔限制。
    pub fn set_min_interval_ticks(&mut self, min_interval_ticks: u64) {
        self.min_interval_ticks = min_interval_ticks;
    }

    /// 暂停调度，保留队列但不再拉起任务。
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// 恢复调度。
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// 读取暂停状态。
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// 按请求 ID 查询前台请求快照。
    pub fn get_request(&self, request_id: u64) -> Option<&ForegroundSelectionRequest> {
        self.requests.get(&request_id)
    }

    /// 获取当前调度进度。
    pub fn get_progress(&self, limit: usize) -> BackgroundProgress {
        let queued = self.foreground_queue.len() + self.background_queue.len();
        let active = self.running.len();
        let processed = self.processed_count;
        let total = processed + active + queued;

        let has_running_or_queued = !self.foreground_queue.is_empty()
            || !self.background_queue.is_empty()
            || !self.running.is_empty();

        let status = if self.paused {
            QueueStatus::Paused
        } else if has_running_or_queued {
            if self.running.is_empty() {
                QueueStatus::Idle
            } else {
                QueueStatus::Running
            }
        } else {
            QueueStatus::Completed
        };

        BackgroundProgress {
            status,
            processed,
            total,
            active,
            queued,
            max_concurrency: self.max_concurrency,
            limit,
            min_interval_ticks: self.min_interval_ticks,
        }
    }

    /// 将后台扫描任务入队。若同 key 已存在则复用 task id，避免重复分析。
    pub fn enqueue_background_task(&mut self, task: BackgroundScanTask) -> u64 {
        let key = task.artifact_key.clone();

        if let Some(task_id) = self.task_by_key.get(&key) {
            return *task_id;
        }

        let task_id = self.next_task_id;
        self.next_task_id += 1;

        self.task_by_key.insert(key, task_id);
        self.tasks.insert(task_id, task);
        self.background_queue.push_back(task_id);

        task_id
    }

    /// 将前台 selection 请求入队。
    ///
    /// 约束：
    /// - 缓存命中直接返回 cache-hit。
    /// - 同 key 同 generation 的待完成请求会合并返回，避免重复建 task。
    pub fn enqueue_foreground_request(
        &mut self,
        artifact_key: AnalysisArtifactKey,
        generation: u64,
        processing_ticks: u64,
    ) -> ForegroundSelectionEnqueueResult {
        if let Some(cached_artifact) = self.artifact_cache.get(&artifact_key).cloned() {
            return self.record_cache_hit(artifact_key, generation, cached_artifact);
        }

        if let Some(existing_request_id) = self.latest_request_by_key.get(&artifact_key).copied() {
            let existing_generation = self
                .latest_generation_by_key
                .get(&artifact_key)
                .copied()
                .unwrap_or(0);

            if generation <= existing_generation {
                let is_same_generation_pending = self
                    .requests
                    .get(&existing_request_id)
                    .is_some_and(|request| {
                        request.state == ForegroundRequestState::Pending
                            && request.generation == generation
                    });

                if is_same_generation_pending {
                    let existing_task_id = self
                        .requests
                        .get(&existing_request_id)
                        .map(|request| request.linked_task_id)
                        .filter(|task_id| *task_id > 0);

                    return ForegroundSelectionEnqueueResult {
                        request_id: existing_request_id,
                        task_id: existing_task_id,
                        merged_with: Some(existing_request_id),
                        cache_hit: false,
                    };
                }
            }

            if let Some(existing_request) = self.requests.get_mut(&existing_request_id) {
                if existing_request.state == ForegroundRequestState::Pending {
                    existing_request.state = ForegroundRequestState::Stale;
                }
            }
        }

        let request_id = self.next_request_id;
        self.next_request_id += 1;

        let task_id =
            self.ensure_foreground_task(artifact_key.clone(), generation, processing_ticks);

        let request = ForegroundSelectionRequest {
            request_id,
            artifact_key: artifact_key.clone(),
            generation,
            linked_task_id: task_id,
            state: ForegroundRequestState::Pending,
            artifact: None,
        };

        self.requests.insert(request_id, request);
        self.latest_generation_by_key
            .insert(artifact_key.clone(), generation);
        self.latest_request_by_key
            .insert(artifact_key.clone(), request_id);
        self.request_waiting_by_key.insert(artifact_key, request_id);

        ForegroundSelectionEnqueueResult {
            request_id,
            task_id: Some(task_id),
            merged_with: None,
            cache_hit: false,
        }
    }

    /// 逻辑时钟推进一次调度并返回本轮变更结果。
    pub fn tick(&mut self, tick: u64, limit: usize) -> OrchestratorTickResult {
        let mut result = OrchestratorTickResult {
            started_task_ids: Vec::new(),
            started_task_descriptors: Vec::new(),
            completed_task_ids: Vec::new(),
            completed_task_descriptors: Vec::new(),
            completed_request_ids: Vec::new(),
            background_progress: BackgroundProgress {
                status: QueueStatus::Idle,
                processed: 0,
                total: 0,
                active: 0,
                queued: 0,
                max_concurrency: self.max_concurrency,
                limit,
                min_interval_ticks: self.min_interval_ticks,
            },
        };

        let now = tick.max(self.now_tick);
        self.now_tick = now;

        self.complete_tasks(now, &mut result);
        self.start_tasks(now, limit, &mut result);
        result.background_progress = self.get_progress(limit);

        result
    }

    /// 在测试中注入缓存产物，直接触发 cache-hit 分支。
    pub fn seed_artifact_for_test(&mut self, artifact: AnalysisArtifact) {
        self.artifact_cache.insert(artifact.key.clone(), artifact);
    }

    /// 查询缓存产物是否已可用。
    pub fn take_cached_artifact(&self, key: &AnalysisArtifactKey) -> Option<&AnalysisArtifact> {
        self.artifact_cache.get(key)
    }

    fn record_cache_hit(
        &mut self,
        artifact_key: AnalysisArtifactKey,
        generation: u64,
        cached_artifact: AnalysisArtifact,
    ) -> ForegroundSelectionEnqueueResult {
        let request_id = self.next_request_id;
        self.next_request_id += 1;

        let request = ForegroundSelectionRequest {
            request_id,
            artifact_key: artifact_key.clone(),
            generation,
            linked_task_id: 0,
            state: ForegroundRequestState::CacheHit,
            artifact: Some(cached_artifact),
        };

        self.requests.insert(request_id, request);
        self.latest_generation_by_key
            .insert(artifact_key.clone(), generation);
        self.latest_request_by_key.insert(artifact_key, request_id);

        ForegroundSelectionEnqueueResult {
            request_id,
            task_id: None,
            merged_with: None,
            cache_hit: true,
        }
    }

    fn ensure_foreground_task(
        &mut self,
        artifact_key: AnalysisArtifactKey,
        generation: u64,
        processing_ticks: u64,
    ) -> u64 {
        if let Some(task_id) = self.task_by_key.get(&artifact_key).copied() {
            let running = self.running.contains_key(&task_id);
            let in_foreground_queue = self.foreground_queue.iter().any(|id| *id == task_id);

            if !running && !in_foreground_queue {
                self.remove_from_queue(task_id);
                self.foreground_queue.push_front(task_id);
            }

            if let Some(task) = self.tasks.get_mut(&task_id) {
                task.priority = AnalysisPriority::Foreground;
                task.generation = generation;
                task.processing_ticks = processing_ticks.max(1);
            }

            return task_id;
        }

        let task_id = self.next_task_id;
        self.next_task_id += 1;

        let task = BackgroundScanTask {
            artifact_key: artifact_key.clone(),
            priority: AnalysisPriority::Foreground,
            generation,
            processing_ticks: processing_ticks.max(1),
        };

        self.task_by_key.insert(artifact_key, task_id);
        self.tasks.insert(task_id, task);
        self.foreground_queue.push_back(task_id);

        task_id
    }

    fn complete_tasks(&mut self, now: u64, result: &mut OrchestratorTickResult) {
        let due_ids: Vec<u64> = self
            .running
            .iter()
            .filter(|(_, running_task)| running_task.complete_at <= now)
            .map(|(task_id, _)| *task_id)
            .collect();

        for task_id in due_ids {
            let Some(running) = self.running.remove(&task_id) else {
                continue;
            };
            let task = running.task;

            self.processed_count += 1;
            result.completed_task_ids.push(task_id);
            result.completed_task_descriptors
                .push(task_to_orchestrator_descriptor(task_id, &task));

            let artifact = AnalysisArtifact {
                key: task.artifact_key.clone(),
                sequence: task.generation,
                ready_tick: now,
            };

            self.artifact_cache
                .insert(task.artifact_key.clone(), artifact.clone());
            self.task_by_key.remove(&task.artifact_key);
            self.tasks.remove(&task_id);
            self.complete_request_if_waiting(&artifact, result);
        }
    }

    fn complete_request_if_waiting(
        &mut self,
        artifact: &AnalysisArtifact,
        result: &mut OrchestratorTickResult,
    ) {
        let request_id = match self.request_waiting_by_key.get(&artifact.key).copied() {
            Some(id) => id,
            None => return,
        };

        let Some(request) = self.requests.get_mut(&request_id) else {
            self.request_waiting_by_key.remove(&artifact.key);
            return;
        };

        let latest_generation = self
            .latest_generation_by_key
            .get(&artifact.key)
            .copied()
            .unwrap_or_default();

        if request.state == ForegroundRequestState::Pending
            && request.generation == latest_generation
        {
            request.state = ForegroundRequestState::Completed;
            request.artifact = Some(artifact.clone());
            result.completed_request_ids.push(request_id);
        }

        self.request_waiting_by_key.remove(&artifact.key);
    }

    fn start_tasks(&mut self, now: u64, limit: usize, result: &mut OrchestratorTickResult) {
        if self.paused || limit == 0 {
            return;
        }

        let mut started = 0;

        while started < limit {
            if self.running.len() >= self.max_concurrency {
                break;
            }

            if now < self.next_start_tick {
                break;
            }

            let task_id = if let Some(task_id) = self.foreground_queue.pop_front() {
                task_id
            } else {
                match self.background_queue.pop_front() {
                    Some(id) => id,
                    None => break,
                }
            };

            let Some(task) = self.tasks.get(&task_id).cloned() else {
                continue;
            };

            let completion_tick = now + task.processing_ticks.max(1);
            let task_descriptor = task_to_orchestrator_descriptor(task_id, &task);
            self.running.insert(
                task_id,
                RunningTask {
                    task: task.clone(),
                    complete_at: completion_tick,
                },
            );

            result.started_task_ids.push(task_id);
            result
                .started_task_descriptors
                .push(task_descriptor);
            started += 1;

            if self.min_interval_ticks > 0 {
                self.next_start_tick = now + self.min_interval_ticks;
            }
        }
    }

    fn remove_from_queue(&mut self, task_id: u64) {
        if let Some(pos) = self.foreground_queue.iter().position(|id| *id == task_id) {
            self.foreground_queue.remove(pos);
            return;
        }

        if let Some(pos) = self.background_queue.iter().position(|id| *id == task_id) {
            self.background_queue.remove(pos);
        }
    }
}

fn task_to_orchestrator_descriptor(
    task_id: u64,
    task: &BackgroundScanTask,
) -> OrchestratorTaskDescriptor {
    OrchestratorTaskDescriptor {
        task_id,
        source_path: task.artifact_key.source_path.clone(),
        project_name: task.artifact_key.project_name.clone(),
        file_id: task.artifact_key.file_id.clone(),
        revision: task.artifact_key.revision.clone(),
        scope: task.artifact_key.scope,
        generation: task.generation,
        active_component_id: task.artifact_key.active_component_id.clone(),
        selected_component_ids: task.artifact_key.selected_component_ids.clone(),
    }
}
