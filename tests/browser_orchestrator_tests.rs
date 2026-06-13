use metadata_checker::browser_orchestrator::{
    AnalysisArtifact, AnalysisArtifactKey, AnalysisArtifactScope, AnalysisPriority,
    BackgroundScanTask, BrowserAnalysisOrchestrator, ForegroundRequestState, QueueStatus,
};

fn foreground_key(name: &str, seq: u64) -> AnalysisArtifactKey {
    AnalysisArtifactKey::foreground(
        Some("project".to_string()),
        format!("app/{name}.spg"),
        Some("file-id".to_string()),
        Some("r1".to_string()),
        Some(format!("c{seq}")),
        vec![format!("selected-{seq}")],
    )
}

fn background_key(name: &str) -> AnalysisArtifactKey {
    AnalysisArtifactKey::background(
        Some("project".to_string()),
        format!("app/{name}.spg"),
        Some("file-id".to_string()),
        Some("r1".to_string()),
    )
}

#[test]
fn test_foreground_request_waiter_completes_without_waiting_unrelated_background() {
    let mut orchestrator = BrowserAnalysisOrchestrator::new(2, 0);

    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("background-a"),
        priority: AnalysisPriority::Background,
        generation: 1,
        processing_ticks: 2,
    });
    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("background-b"),
        priority: AnalysisPriority::Background,
        generation: 2,
        processing_ticks: 4,
    });

    // 两个 background 任务同时启动。
    let first_tick = orchestrator.tick(1, 2);
    assert_eq!(first_tick.started_task_ids.len(), 2);

    let fg_key = foreground_key("foreground", 1);
    let fg = orchestrator.enqueue_foreground_request(fg_key.clone(), 1, 1);
    assert!(fg.request_id > 0);
    assert_eq!(fg.cache_hit, false);

    // B 仍在运行，foreground F 入队后不应被阻塞。
    let second_tick = orchestrator.tick(3, 2);
    assert!(
        second_tick
            .started_task_ids
            .contains(&fg.task_id.expect("foreground task should be scheduled"))
    );

    let third_tick = orchestrator.tick(4, 2);
    assert!(third_tick.completed_request_ids.contains(&fg.request_id));

    let progress = orchestrator.get_progress(2);
    assert_eq!(progress.active, 1);
    assert_eq!(progress.status, QueueStatus::Running);
    assert!(progress.total >= progress.processed);
}

#[test]
fn test_tick_returns_started_task_descriptors_with_artifact_keys() {
    let mut orchestrator = BrowserAnalysisOrchestrator::new(2, 0);

    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("descriptor"),
        priority: AnalysisPriority::Background,
        generation: 7,
        processing_ticks: 1,
    });

    let tick = orchestrator.tick(1, 2);
    assert_eq!(tick.started_task_ids.len(), 1);
    assert_eq!(tick.started_task_descriptors.len(), 1);
    let descriptor = &tick.started_task_descriptors[0];
    assert_eq!(descriptor.source_path, "app/descriptor.spg");
    assert_eq!(descriptor.project_name, Some("project".to_string()));
    assert_eq!(descriptor.file_id, Some("file-id".to_string()));
    assert_eq!(descriptor.revision, Some("r1".to_string()));
    assert_eq!(descriptor.scope, AnalysisArtifactScope::Background);
    assert_eq!(descriptor.generation, 7);
    assert_eq!(descriptor.active_component_id, None);
    assert_eq!(descriptor.selected_component_ids, Vec::<String>::new());
}

#[test]
fn test_tick_returns_completed_task_descriptors_for_artifact_lookup() {
    let mut orchestrator = BrowserAnalysisOrchestrator::new(1, 0);

    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("descriptor-complete"),
        priority: AnalysisPriority::Background,
        generation: 8,
        processing_ticks: 1,
    });

    let _ = orchestrator.tick(1, 1);
    let tick = orchestrator.tick(2, 1);
    assert_eq!(tick.completed_task_ids.len(), 1);
    assert_eq!(tick.completed_task_descriptors.len(), 1);
    assert_eq!(
        tick.completed_task_descriptors[0].source_path,
        "app/descriptor-complete.spg"
    );
    assert_eq!(tick.completed_task_descriptors[0].generation, 8);
}

#[test]
fn test_foreground_priority_over_queued_background() {
    let mut orchestrator = BrowserAnalysisOrchestrator::new(1, 0);

    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("bg-1"),
        priority: AnalysisPriority::Background,
        generation: 1,
        processing_ticks: 3,
    });
    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("bg-2"),
        priority: AnalysisPriority::Background,
        generation: 2,
        processing_ticks: 3,
    });

    let fg_key = foreground_key("foreground", 2);
    let fg_request = orchestrator.enqueue_foreground_request(fg_key, 2, 1);

    let result = orchestrator.tick(1, 2);
    assert_eq!(result.started_task_ids, vec![fg_request.task_id.unwrap()]);
}

#[test]
fn test_pause_and_resume_keeps_order_and_restarts_later() {
    let mut orchestrator = BrowserAnalysisOrchestrator::new(1, 0);

    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("first"),
        priority: AnalysisPriority::Background,
        generation: 1,
        processing_ticks: 3,
    });
    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("second"),
        priority: AnalysisPriority::Background,
        generation: 2,
        processing_ticks: 1,
    });

    orchestrator.tick(1, 1);
    orchestrator.pause();

    let paused_tick = orchestrator.tick(2, 1);
    assert_eq!(paused_tick.started_task_ids.len(), 0);

    orchestrator.resume();
    let resumed_tick = orchestrator.tick(4, 1);
    assert_eq!(resumed_tick.started_task_ids.len(), 1);
    assert_eq!(
        resumed_tick.background_progress.status,
        QueueStatus::Running
    );

    let again = orchestrator.tick(5, 2);
    assert!(again.background_progress.processed >= 1);
    assert_eq!(orchestrator.get_progress(2).status, QueueStatus::Completed);
}

#[test]
fn test_processed_count_never_exceeds_total_and_active_single_flight() {
    let mut orchestrator = BrowserAnalysisOrchestrator::new(2, 0);

    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("one"),
        priority: AnalysisPriority::Background,
        generation: 1,
        processing_ticks: 2,
    });
    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("two"),
        priority: AnalysisPriority::Background,
        generation: 2,
        processing_ticks: 2,
    });
    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("three"),
        priority: AnalysisPriority::Background,
        generation: 3,
        processing_ticks: 2,
    });

    for tick in 1..6 {
        let result = orchestrator.tick(tick, 3);
        assert!(result.background_progress.processed <= result.background_progress.total);
        assert!(result.background_progress.active <= orchestrator.get_progress(3).max_concurrency);
    }

    let final_progress = orchestrator.get_progress(3);
    assert!(final_progress.processed >= 3);
    assert!(final_progress.total >= final_progress.processed);
    assert_eq!(final_progress.status, QueueStatus::Completed);
}

#[test]
fn test_cache_hit_and_duplicate_foreground_merge() {
    let mut orchestrator = BrowserAnalysisOrchestrator::new(1, 0);
    let key = foreground_key("cached", 3);

    orchestrator.seed_artifact_for_test(AnalysisArtifact {
        key: key.clone(),
        sequence: 9,
        ready_tick: 5,
    });

    let cached = orchestrator.enqueue_foreground_request(key.clone(), 10, 1);
    assert!(cached.cache_hit);
    let request = orchestrator
        .get_request(cached.request_id)
        .expect("request should be stored");
    assert_eq!(request.state, ForegroundRequestState::CacheHit);
    assert!(request.artifact.is_some());

    let duplicate = orchestrator.enqueue_foreground_request(key.clone(), 10, 1);
    assert_eq!(duplicate.task_id, cached.task_id);
    assert_eq!(duplicate.merged_with, None);
}

#[test]
fn test_duplicate_pending_foreground_request_reuses_task() {
    let mut orchestrator = BrowserAnalysisOrchestrator::new(2, 0);
    let key = foreground_key("pending-merge", 7);

    let first = orchestrator.enqueue_foreground_request(key.clone(), 1, 5);
    let first_task_id = orchestrator
        .get_request(first.request_id)
        .expect("first request should be stored")
        .linked_task_id;
    assert!(first_task_id > 0);

    let second = orchestrator.enqueue_foreground_request(key.clone(), 1, 5);
    assert_eq!(second.request_id, first.request_id);
    assert_eq!(second.task_id, Some(first_task_id));
    assert_eq!(second.merged_with, Some(first.request_id));

    let first_tick = orchestrator.tick(1, 1);
    assert_eq!(first_tick.started_task_ids, vec![first_task_id]);

    let second_tick = orchestrator.tick(6, 1);
    assert_eq!(second_tick.completed_request_ids, vec![first.request_id]);
}

#[test]
fn test_stale_foreground_generation_does_not_override_latest() {
    let mut orchestrator = BrowserAnalysisOrchestrator::new(1, 0);

    let key = AnalysisArtifactKey::foreground(
        Some("project".to_string()),
        "app/stale.spg",
        Some("file-id".to_string()),
        Some("r1".to_string()),
        Some("c".to_string()),
        vec!["selected".to_string()],
    );

    let first = orchestrator.enqueue_foreground_request(key.clone(), 1, 3);
    let second = orchestrator.enqueue_foreground_request(key.clone(), 2, 1);

    assert_ne!(first.request_id, second.request_id);

    orchestrator.tick(1, 1);
    orchestrator.tick(2, 1);

    let first_request = orchestrator
        .get_request(first.request_id)
        .expect("first request should exist");
    let second_request = orchestrator
        .get_request(second.request_id)
        .expect("second request should exist");

    assert_eq!(second_request.state, ForegroundRequestState::Completed);
    let artifact = second_request
        .artifact
        .as_ref()
        .expect("newest request should carry artifact");
    assert_eq!(artifact.sequence, 2);
    assert_eq!(first_request.state, ForegroundRequestState::Stale);
}

#[test]
fn test_queue_progress_invariants_for_mixed_foreground_background() {
    let mut orchestrator = BrowserAnalysisOrchestrator::new(2, 0);

    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("bg-1"),
        priority: AnalysisPriority::Background,
        generation: 1,
        processing_ticks: 4,
    });
    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("bg-2"),
        priority: AnalysisPriority::Background,
        generation: 2,
        processing_ticks: 4,
    });
    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("bg-3"),
        priority: AnalysisPriority::Background,
        generation: 3,
        processing_ticks: 4,
    });

    for tick in 1..=7 {
        if tick == 2 {
            let fg = orchestrator.enqueue_foreground_request(foreground_key("mix-fg", 1), 1, 2);
            assert!(fg.task_id.is_some());
        }

        if tick == 4 {
            let first = orchestrator.enqueue_foreground_request(foreground_key("mix-fg", 1), 1, 2);
            let second = orchestrator.enqueue_foreground_request(foreground_key("mix-fg", 1), 1, 2);
            assert_eq!(first.task_id, second.task_id);
        }

        if tick == 5 {
            let fg = orchestrator.enqueue_foreground_request(foreground_key("mix-fg-2", 2), 2, 2);
            assert!(fg.task_id.is_some());
        }

        let result = orchestrator.tick(tick, 3);
        assert!(
            result.background_progress.active <= result.background_progress.max_concurrency,
            "active should always be bounded by max concurrency"
        );
        assert!(
            result.background_progress.processed <= result.background_progress.total,
            "processed should never exceed total tasks"
        );
    }

    let final_progress = orchestrator.get_progress(3);
    if final_progress.status != QueueStatus::Completed {
        let mut progress = final_progress;
        for tick in 8..20 {
            let result = orchestrator.tick(tick, 3);
            assert!(
                result.background_progress.processed <= result.background_progress.total,
                "processed should never exceed total tasks"
            );
            assert!(
                result.background_progress.active <= result.background_progress.max_concurrency,
                "active should always be bounded by max concurrency"
            );
            progress = result.background_progress;
            if progress.status == QueueStatus::Completed {
                break;
            }
        }

        assert_eq!(progress.status, QueueStatus::Completed);
    }

    assert!(orchestrator.get_progress(3).total >= orchestrator.get_progress(3).processed);
}

#[test]
fn test_min_interval_limits_task_start_without_real_sleep() {
    let mut orchestrator = BrowserAnalysisOrchestrator::new(2, 2);

    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("interval-1"),
        priority: AnalysisPriority::Background,
        generation: 1,
        processing_ticks: 1,
    });
    orchestrator.enqueue_background_task(BackgroundScanTask {
        artifact_key: background_key("interval-2"),
        priority: AnalysisPriority::Background,
        generation: 2,
        processing_ticks: 1,
    });

    let first = orchestrator.tick(1, 2);
    assert_eq!(first.started_task_ids.len(), 1);

    let second = orchestrator.tick(1, 2);
    assert_eq!(second.started_task_ids.len(), 0);

    let third = orchestrator.tick(2, 2);
    assert_eq!(third.started_task_ids.len(), 0);

    let fourth = orchestrator.tick(3, 2);
    assert_eq!(fourth.started_task_ids.len(), 1);
}
