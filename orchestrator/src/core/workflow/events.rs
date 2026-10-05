use crate::core::task::{TaskInputMapping, TaskInstance, TaskSatisfactionStatus, TaskStatus};
use crate::core::verifier::VerifierAttemptMetadata;
use crate::core::worker::WorkerHostId;
use crate::core::workflow::models::{
    EarlyCompletion, VerifierFeedbackEntry, VerifierGenerationState, VerifierStateStatus,
    WorkflowInstance, WorkflowStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowInstanceEvent {
    /// Initializes a new workflow instance snapshot.
    WorkflowCreated { instance: WorkflowInstance },
    /// Changes the overall workflow instance status.
    WorkflowStatusChanged { status: WorkflowStatus },
    /// Durably requests early completion; pending attempts are skipped in the same batch.
    EarlyCompletionRequested { metadata: EarlyCompletion },
    /// Resets in-flight running workflow or task state after orchestrator restart.
    StartupRecoveryApplied { task_attempt_ids: Vec<String> },
    /// Adds a concrete task attempt to the workflow instance.
    TaskMaterialized {
        task_attempt_id: String,
        task: TaskInstance,
    },
    /// Changes the status of an existing task attempt.
    TaskStatusChanged {
        task_attempt_id: String,
        status: TaskStatus,
    },
    /// Replaces the input data for an existing task attempt.
    TaskInputDataSet {
        task_attempt_id: String,
        input_data: Vec<serde_json::Value>,
    },
    /// Replaces the upstream input mapping for an existing task attempt.
    TaskInputMappingSet {
        task_attempt_id: String,
        input_mapping: Vec<TaskInputMapping>,
    },
    /// Records or clears output data for an existing task attempt.
    TaskOutputRecorded {
        task_attempt_id: String,
        output_data: Option<serde_json::Value>,
    },
    /// Changes whether a task attempt is pending, satisfied, or unsatisfied.
    TaskSatisfactionChanged {
        task_attempt_id: String,
        satisfaction_status: TaskSatisfactionStatus,
    },
    /// Sets or clears verifier metadata on a task attempt.
    TaskVerifierMetadataSet {
        task_attempt_id: String,
        verifier_metadata: Option<VerifierAttemptMetadata>,
    },
    /// Creates or replaces verifier generation state for a verifier task.
    VerifierStateUpserted {
        verifier_task_id: String,
        state: VerifierGenerationState,
    },
    /// Appends verifier feedback history for a verifier task.
    VerifierFeedbackRecorded {
        verifier_task_id: String,
        feedback: VerifierFeedbackEntry,
    },
    /// Updates verifier state status, selected generation, and exit reason.
    VerifierStateStatusChanged {
        verifier_task_id: String,
        status: VerifierStateStatus,
        selected_generation: Option<u32>,
        exit_reason: Option<String>,
    },
    /// Records user input submitted for a task attempt waiting in InputNeeded.
    HumanInputSubmitted {
        task_attempt_id: String,
        continuation_task_attempt_id: String,
        submitted_input: serde_json::Value,
    },
    /// Requests a failed task attempt to run again without changing workflow placement.
    TaskRetryRequested { task_attempt_id: String },
    /// Requests a failed task attempt to run again, allowing workflow placement reassignment.
    TaskForceRetryRequested {
        task_attempt_id: String,
        previous_host_id: Option<WorkerHostId>,
        target_host_id: WorkerHostId,
        local_context_may_be_lost: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEventRecord {
    pub created_time: u64,
    pub event: WorkflowInstanceEvent,
}

pub fn apply_workflow_instance_event(
    instance: &mut WorkflowInstance,
    event: &WorkflowInstanceEvent,
) -> anyhow::Result<()> {
    match event {
        WorkflowInstanceEvent::WorkflowCreated { .. } => {
            anyhow::bail!("workflow_created can only initialize an empty workflow snapshot")
        }
        WorkflowInstanceEvent::WorkflowStatusChanged { status } => {
            instance.status = status.clone();
        }
        WorkflowInstanceEvent::EarlyCompletionRequested { .. } => {}
        WorkflowInstanceEvent::StartupRecoveryApplied { task_attempt_ids } => {
            if instance.status == WorkflowStatus::Running {
                instance.status = WorkflowStatus::Pending;
            }
            for task_attempt_id in task_attempt_ids {
                let task = task_mut(instance, task_attempt_id)?;
                if task.status == TaskStatus::Running {
                    task.status = TaskStatus::Pending;
                }
            }
        }
        WorkflowInstanceEvent::TaskMaterialized {
            task_attempt_id,
            task,
        } => {
            if instance.tasks.contains_key(task_attempt_id) {
                anyhow::bail!("task attempt {task_attempt_id} already exists");
            }
            instance.tasks.insert(task_attempt_id.clone(), task.clone());
        }
        WorkflowInstanceEvent::TaskStatusChanged {
            task_attempt_id,
            status,
        } => {
            task_mut(instance, task_attempt_id)?.status = status.clone();
        }
        WorkflowInstanceEvent::TaskInputDataSet {
            task_attempt_id,
            input_data,
        } => {
            task_mut(instance, task_attempt_id)?.input_data = input_data.clone();
        }
        WorkflowInstanceEvent::TaskInputMappingSet {
            task_attempt_id,
            input_mapping,
        } => {
            task_mut(instance, task_attempt_id)?.input_mapping = input_mapping.clone();
        }
        WorkflowInstanceEvent::TaskOutputRecorded {
            task_attempt_id,
            output_data,
        } => {
            task_mut(instance, task_attempt_id)?.output_data = output_data.clone();
        }
        WorkflowInstanceEvent::TaskSatisfactionChanged {
            task_attempt_id,
            satisfaction_status,
        } => {
            task_mut(instance, task_attempt_id)?.satisfaction_status = satisfaction_status.clone();
        }
        WorkflowInstanceEvent::TaskVerifierMetadataSet {
            task_attempt_id,
            verifier_metadata,
        } => {
            task_mut(instance, task_attempt_id)?.verifier_metadata = verifier_metadata.clone();
        }
        WorkflowInstanceEvent::VerifierStateUpserted {
            verifier_task_id,
            state,
        } => {
            instance
                .verifier_states
                .insert(verifier_task_id.clone(), state.clone());
        }
        WorkflowInstanceEvent::VerifierFeedbackRecorded {
            verifier_task_id,
            feedback,
        } => {
            let state = verifier_state_mut(instance, verifier_task_id)?;
            state.feedback_history.push(feedback.clone());
        }
        WorkflowInstanceEvent::VerifierStateStatusChanged {
            verifier_task_id,
            status,
            selected_generation,
            exit_reason,
        } => {
            let state = verifier_state_mut(instance, verifier_task_id)?;
            state.status = status.clone();
            state.selected_generation = *selected_generation;
            state.exit_reason = exit_reason.clone();
        }
        WorkflowInstanceEvent::HumanInputSubmitted {
            task_attempt_id,
            continuation_task_attempt_id,
            submitted_input,
        } => {
            let (task_def_id, continuation_generation) = {
                let task = task_mut(instance, task_attempt_id)?;
                if !matches!(task.status, TaskStatus::InputNeeded { .. }) {
                    anyhow::bail!("task attempt {task_attempt_id} is not waiting for input");
                }
                (task.task_def_id.clone(), task.generation_index + 1)
            };
            let expected_continuation_task_attempt_id =
                TaskInstance::make_task_attempt_id(&task_def_id, continuation_generation);
            if *continuation_task_attempt_id != expected_continuation_task_attempt_id {
                anyhow::bail!(
                    "human input continuation for {task_attempt_id} must be {expected_continuation_task_attempt_id}"
                );
            }
            let continuation = TaskInstance {
                task_def_id,
                status: TaskStatus::Pending,
                satisfaction_status: TaskSatisfactionStatus::Pending,
                human_input: Some(submitted_input.clone()),
                input_data: vec![],
                input_mapping: vec![],
                output_data: None,
                generation_index: continuation_generation,
                verifier_metadata: None,
            };
            if instance.tasks.contains_key(continuation_task_attempt_id) {
                anyhow::bail!("task attempt {continuation_task_attempt_id} already exists");
            }
            instance
                .tasks
                .insert(continuation_task_attempt_id.clone(), continuation);
            instance.status = WorkflowStatus::Pending;
        }
        WorkflowInstanceEvent::TaskRetryRequested { task_attempt_id } => {
            reset_failed_task_for_retry(instance, task_attempt_id)?;
        }
        WorkflowInstanceEvent::TaskForceRetryRequested {
            task_attempt_id,
            previous_host_id,
            target_host_id,
            local_context_may_be_lost,
        } => {
            if &instance.pinned_worker_host != previous_host_id {
                anyhow::bail!("force retry previous host does not match workflow pin");
            }
            let expected_context_may_be_lost = previous_host_id.as_ref() != Some(target_host_id);
            if *local_context_may_be_lost != expected_context_may_be_lost {
                anyhow::bail!("force retry local context loss flag does not match host change");
            }

            reset_failed_task_for_retry(instance, task_attempt_id)?;
            instance.pinned_worker_host = Some(target_host_id.clone());
        }
    }

    Ok(())
}

pub fn changed_task_attempt_ids(events: &[WorkflowEventRecord]) -> HashSet<String> {
    let mut task_attempt_ids = HashSet::new();
    for record in events {
        match &record.event {
            WorkflowInstanceEvent::WorkflowCreated { instance } => {
                task_attempt_ids.extend(instance.tasks.keys().cloned());
            }
            WorkflowInstanceEvent::WorkflowStatusChanged { .. }
            | WorkflowInstanceEvent::EarlyCompletionRequested { .. }
            | WorkflowInstanceEvent::VerifierStateUpserted { .. }
            | WorkflowInstanceEvent::VerifierFeedbackRecorded { .. }
            | WorkflowInstanceEvent::VerifierStateStatusChanged { .. } => {}
            WorkflowInstanceEvent::StartupRecoveryApplied {
                task_attempt_ids: recovered_task_attempt_ids,
            } => {
                task_attempt_ids.extend(recovered_task_attempt_ids.iter().cloned());
            }
            WorkflowInstanceEvent::TaskMaterialized {
                task_attempt_id, ..
            }
            | WorkflowInstanceEvent::TaskStatusChanged {
                task_attempt_id, ..
            }
            | WorkflowInstanceEvent::TaskInputDataSet {
                task_attempt_id, ..
            }
            | WorkflowInstanceEvent::TaskInputMappingSet {
                task_attempt_id, ..
            }
            | WorkflowInstanceEvent::TaskOutputRecorded {
                task_attempt_id, ..
            }
            | WorkflowInstanceEvent::TaskSatisfactionChanged {
                task_attempt_id, ..
            }
            | WorkflowInstanceEvent::TaskVerifierMetadataSet {
                task_attempt_id, ..
            }
            | WorkflowInstanceEvent::TaskRetryRequested { task_attempt_id }
            | WorkflowInstanceEvent::TaskForceRetryRequested {
                task_attempt_id, ..
            } => {
                task_attempt_ids.insert(task_attempt_id.clone());
            }
            WorkflowInstanceEvent::HumanInputSubmitted {
                continuation_task_attempt_id,
                ..
            } => {
                task_attempt_ids.insert(continuation_task_attempt_id.clone());
            }
        }
    }
    task_attempt_ids
}

pub fn reduce_workflow_instance_events(
    current: Option<WorkflowInstance>,
    events: &[WorkflowInstanceEvent],
) -> anyhow::Result<WorkflowInstance> {
    let mut iter = events.iter();
    let mut instance = match current {
        Some(instance) => instance,
        None => match iter.next() {
            Some(WorkflowInstanceEvent::WorkflowCreated { instance }) => instance.clone(),
            Some(_) => anyhow::bail!("first event for a missing workflow snapshot must create it"),
            None => anyhow::bail!("event batch must not be empty"),
        },
    };

    for event in iter {
        apply_workflow_instance_event(&mut instance, event)?;
    }

    Ok(instance)
}

fn task_mut<'a>(
    instance: &'a mut WorkflowInstance,
    task_attempt_id: &str,
) -> anyhow::Result<&'a mut TaskInstance> {
    instance
        .tasks
        .get_mut(task_attempt_id)
        .ok_or_else(|| anyhow::anyhow!("task attempt {task_attempt_id} not found"))
}

fn reset_failed_task_for_retry(
    instance: &mut WorkflowInstance,
    task_attempt_id: &str,
) -> anyhow::Result<()> {
    if instance.status != WorkflowStatus::Failed {
        anyhow::bail!("workflow instance is not failed");
    }

    {
        let task = task_mut(instance, task_attempt_id)?;
        if task.status != TaskStatus::Failed {
            anyhow::bail!("task attempt {task_attempt_id} is not failed");
        }

        task.status = TaskStatus::Pending;
        task.satisfaction_status = TaskSatisfactionStatus::Pending;
        task.output_data = None;
        task.verifier_metadata = None;
    }

    instance.status = WorkflowStatus::Pending;
    Ok(())
}

fn verifier_state_mut<'a>(
    instance: &'a mut WorkflowInstance,
    verifier_task_id: &str,
) -> anyhow::Result<&'a mut VerifierGenerationState> {
    instance
        .verifier_states
        .get_mut(verifier_task_id)
        .ok_or_else(|| anyhow::anyhow!("verifier state {verifier_task_id} not found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::task::TaskSatisfactionStatus;
    use std::collections::HashMap;

    fn instance() -> WorkflowInstance {
        WorkflowInstance {
            id: "wf-1".to_string(),
            workflow_def_id: "wf".to_string(),
            version: 0,
            status: WorkflowStatus::Pending,
            trigger_input: None,
            pinned_worker_host: None,
            tasks: HashMap::new(),
            verifier_states: HashMap::new(),
        }
    }

    fn task() -> TaskInstance {
        TaskInstance {
            task_def_id: "task-a".to_string(),
            status: TaskStatus::Pending,
            satisfaction_status: TaskSatisfactionStatus::Pending,
            human_input: None,
            input_data: vec![],
            input_mapping: vec![],
            output_data: None,
            generation_index: 1,
            verifier_metadata: None,
        }
    }

    #[test]
    fn reducer_initializes_workflow_from_create_event() {
        let reduced = reduce_workflow_instance_events(
            None,
            &[WorkflowInstanceEvent::WorkflowCreated {
                instance: instance(),
            }],
        )
        .unwrap();

        assert_eq!(reduced.id, "wf-1");
        assert_eq!(reduced.status, WorkflowStatus::Pending);
    }

    #[test]
    fn reducer_applies_ordered_task_events() {
        let instance = reduce_workflow_instance_events(
            Some(instance()),
            &[
                WorkflowInstanceEvent::TaskMaterialized {
                    task_attempt_id: "task-a[1]".to_string(),
                    task: task(),
                },
                WorkflowInstanceEvent::TaskStatusChanged {
                    task_attempt_id: "task-a[1]".to_string(),
                    status: TaskStatus::Running,
                },
                WorkflowInstanceEvent::TaskStatusChanged {
                    task_attempt_id: "task-a[1]".to_string(),
                    status: TaskStatus::Completed,
                },
                WorkflowInstanceEvent::TaskStatusChanged {
                    task_attempt_id: "task-a[1]".to_string(),
                    status: TaskStatus::Failed,
                },
            ],
        )
        .unwrap();

        assert_eq!(instance.tasks["task-a[1]"].status, TaskStatus::Failed);
    }

    #[test]
    fn reducer_rejects_missing_task_updates() {
        let mut instance = instance();
        let result = apply_workflow_instance_event(
            &mut instance,
            &WorkflowInstanceEvent::TaskStatusChanged {
                task_attempt_id: "task-a[1]".to_string(),
                status: TaskStatus::Running,
            },
        );

        assert!(result.is_err());
    }

    #[test]
    fn reducer_applies_verifier_events() {
        let instance = reduce_workflow_instance_events(
            Some(instance()),
            &[
                WorkflowInstanceEvent::VerifierStateUpserted {
                    verifier_task_id: "verify".to_string(),
                    state: VerifierGenerationState {
                        verifier_task_id: "verify".to_string(),
                        rerun_start_task_id: "task-a".to_string(),
                        latest_generation: 1,
                        selected_generation: None,
                        feedback_history: vec![],
                        status: VerifierStateStatus::Running,
                        exit_reason: None,
                    },
                },
                WorkflowInstanceEvent::VerifierFeedbackRecorded {
                    verifier_task_id: "verify".to_string(),
                    feedback: VerifierFeedbackEntry {
                        generation_index: 1,
                        feedback: "retry".to_string(),
                        verifier_output: serde_json::json!({"decision": "continue"}),
                    },
                },
                WorkflowInstanceEvent::VerifierStateStatusChanged {
                    verifier_task_id: "verify".to_string(),
                    status: VerifierStateStatus::Accepted,
                    selected_generation: Some(1),
                    exit_reason: None,
                },
            ],
        )
        .unwrap();

        let state = &instance.verifier_states["verify"];
        assert_eq!(state.status, VerifierStateStatus::Accepted);
        assert_eq!(state.feedback_history.len(), 1);
        assert_eq!(state.selected_generation, Some(1));
    }

    #[test]
    fn reducer_applies_startup_recovery() {
        let mut instance = instance();
        instance.status = WorkflowStatus::Running;
        instance.tasks.insert(
            "task-a[1]".to_string(),
            TaskInstance {
                status: TaskStatus::Running,
                ..task()
            },
        );

        apply_workflow_instance_event(
            &mut instance,
            &WorkflowInstanceEvent::StartupRecoveryApplied {
                task_attempt_ids: vec!["task-a[1]".to_string()],
            },
        )
        .unwrap();

        assert_eq!(instance.status, WorkflowStatus::Pending);
        assert_eq!(instance.tasks["task-a[1]"].status, TaskStatus::Pending);
    }

    #[test]
    fn changed_task_ids_are_extracted_without_reducing_events() {
        let records = vec![
            WorkflowEventRecord {
                created_time: 1,
                event: WorkflowInstanceEvent::TaskStatusChanged {
                    task_attempt_id: "task-a[1]".to_string(),
                    status: TaskStatus::Running,
                },
            },
            WorkflowEventRecord {
                created_time: 2,
                event: WorkflowInstanceEvent::HumanInputSubmitted {
                    task_attempt_id: "task-b[1]".to_string(),
                    continuation_task_attempt_id: "task-b[2]".to_string(),
                    submitted_input: serde_json::json!("approved"),
                },
            },
            WorkflowEventRecord {
                created_time: 3,
                event: WorkflowInstanceEvent::StartupRecoveryApplied {
                    task_attempt_ids: vec!["task-c[1]".to_string()],
                },
            },
            WorkflowEventRecord {
                created_time: 4,
                event: WorkflowInstanceEvent::WorkflowStatusChanged {
                    status: WorkflowStatus::Running,
                },
            },
        ];

        assert_eq!(
            changed_task_attempt_ids(&records),
            HashSet::from([
                "task-a[1]".to_string(),
                "task-b[2]".to_string(),
                "task-c[1]".to_string(),
            ])
        );
    }

    #[test]
    fn reducer_materializes_human_input_continuation() {
        let mut instance = instance();
        instance.status = WorkflowStatus::InputNeeded;
        instance.tasks.insert(
            "task-a[1]".to_string(),
            TaskInstance {
                status: TaskStatus::InputNeeded {
                    input_request: "need input".to_string(),
                },
                ..task()
            },
        );

        apply_workflow_instance_event(
            &mut instance,
            &WorkflowInstanceEvent::HumanInputSubmitted {
                task_attempt_id: "task-a[1]".to_string(),
                continuation_task_attempt_id: "task-a[2]".to_string(),
                submitted_input: serde_json::json!({"answer": "ship it"}),
            },
        )
        .unwrap();

        assert_eq!(instance.status, WorkflowStatus::Pending);
        assert!(matches!(
            instance.tasks["task-a[1]"].status,
            TaskStatus::InputNeeded { .. }
        ));
        assert_eq!(instance.tasks["task-a[2]"].task_def_id, "task-a");
        assert_eq!(instance.tasks["task-a[2]"].status, TaskStatus::Pending);
        assert_eq!(instance.tasks["task-a[2]"].generation_index, 2);
        assert_eq!(
            instance.tasks["task-a[2]"].human_input,
            Some(serde_json::json!({"answer": "ship it"}))
        );
    }

    #[test]
    fn reducer_applies_task_retry_request() {
        let mut instance = instance();
        instance.status = WorkflowStatus::Failed;
        instance.pinned_worker_host = Some(WorkerHostId::new("host-a"));
        instance.tasks.insert(
            "task-a[1]".to_string(),
            TaskInstance {
                status: TaskStatus::Failed,
                satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                input_data: vec![serde_json::json!({"input": true})],
                output_data: Some(serde_json::json!({"stale": true})),
                verifier_metadata: Some(VerifierAttemptMetadata {
                    status: crate::core::verifier::VerifierAttemptStatus::Invalid,
                    decision: None,
                    feedback: None,
                    verifier_output: Some(serde_json::json!({"bad": true})),
                    exit_reason: Some("invalid".to_string()),
                }),
                ..task()
            },
        );

        apply_workflow_instance_event(
            &mut instance,
            &WorkflowInstanceEvent::TaskRetryRequested {
                task_attempt_id: "task-a[1]".to_string(),
            },
        )
        .unwrap();

        let task = &instance.tasks["task-a[1]"];
        assert_eq!(instance.status, WorkflowStatus::Pending);
        assert_eq!(
            instance.pinned_worker_host,
            Some(WorkerHostId::new("host-a"))
        );
        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.satisfaction_status, TaskSatisfactionStatus::Pending);
        assert_eq!(task.output_data, None);
        assert_eq!(task.verifier_metadata, None);
    }

    #[test]
    fn reducer_rejects_retry_for_non_failed_task() {
        let mut instance = instance();
        instance.status = WorkflowStatus::Failed;
        instance
            .tasks
            .insert("task-a[1]".to_string(), TaskInstance { ..task() });

        let error = apply_workflow_instance_event(
            &mut instance,
            &WorkflowInstanceEvent::TaskRetryRequested {
                task_attempt_id: "task-a[1]".to_string(),
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("is not failed"));
    }

    #[test]
    fn reducer_applies_force_retry_request_with_same_host() {
        let mut instance = instance();
        instance.status = WorkflowStatus::Failed;
        instance.pinned_worker_host = Some(WorkerHostId::new("host-a"));
        instance.tasks.insert(
            "task-a[1]".to_string(),
            TaskInstance {
                status: TaskStatus::Failed,
                satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                output_data: Some(serde_json::json!({"stale": true})),
                ..task()
            },
        );

        apply_workflow_instance_event(
            &mut instance,
            &WorkflowInstanceEvent::TaskForceRetryRequested {
                task_attempt_id: "task-a[1]".to_string(),
                previous_host_id: Some(WorkerHostId::new("host-a")),
                target_host_id: WorkerHostId::new("host-a"),
                local_context_may_be_lost: false,
            },
        )
        .unwrap();

        assert_eq!(instance.status, WorkflowStatus::Pending);
        assert_eq!(
            instance.pinned_worker_host,
            Some(WorkerHostId::new("host-a"))
        );
        assert_eq!(instance.tasks["task-a[1]"].status, TaskStatus::Pending);
        assert_eq!(instance.tasks["task-a[1]"].output_data, None);
    }

    #[test]
    fn reducer_applies_force_retry_request_with_reassigned_host() {
        let mut instance = instance();
        instance.status = WorkflowStatus::Failed;
        instance.pinned_worker_host = Some(WorkerHostId::new("host-a"));
        instance.tasks.insert(
            "task-a[1]".to_string(),
            TaskInstance {
                status: TaskStatus::Failed,
                satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                ..task()
            },
        );

        apply_workflow_instance_event(
            &mut instance,
            &WorkflowInstanceEvent::TaskForceRetryRequested {
                task_attempt_id: "task-a[1]".to_string(),
                previous_host_id: Some(WorkerHostId::new("host-a")),
                target_host_id: WorkerHostId::new("host-b"),
                local_context_may_be_lost: true,
            },
        )
        .unwrap();

        assert_eq!(instance.status, WorkflowStatus::Pending);
        assert_eq!(
            instance.pinned_worker_host,
            Some(WorkerHostId::new("host-b"))
        );
        assert_eq!(instance.tasks["task-a[1]"].status, TaskStatus::Pending);
    }

    #[test]
    fn reducer_rejects_force_retry_with_incorrect_context_loss_flag() {
        let mut instance = instance();
        instance.status = WorkflowStatus::Failed;
        instance.pinned_worker_host = Some(WorkerHostId::new("host-a"));
        instance.tasks.insert(
            "task-a[1]".to_string(),
            TaskInstance {
                status: TaskStatus::Failed,
                ..task()
            },
        );

        let error = apply_workflow_instance_event(
            &mut instance,
            &WorkflowInstanceEvent::TaskForceRetryRequested {
                task_attempt_id: "task-a[1]".to_string(),
                previous_host_id: Some(WorkerHostId::new("host-a")),
                target_host_id: WorkerHostId::new("host-b"),
                local_context_may_be_lost: false,
            },
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("local context loss flag does not match")
        );
    }
}
