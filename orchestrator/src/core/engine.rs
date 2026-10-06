use crate::core::function::function_service::resolve_task_function_ref;
use crate::core::namespace::Namespace;
use crate::core::task::{
    ExecutionMetadata, TaskInputMapping, TaskInstance, TaskSatisfactionStatus, TaskStatus,
};
use crate::core::verifier::{
    LoopExecutionContext, LoopFeedbackEntry, VerifierAttemptMetadata, VerifierAttemptStatus,
    VerifierControlConfig, VerifierDecision, VerifierExecutionResult,
};
use crate::core::worker::TaskDispatchConstraints;
use crate::core::workflow::events::WorkflowInstanceEvent;
use crate::core::workflow::models::{
    TaskStatusReport, VerifierFeedbackEntry, VerifierGenerationState, VerifierStateStatus,
    VerifierStatusReport, WorkflowDef, WorkflowInstance, WorkflowStatus, WorkflowStatusReport,
};
use crate::core::workflow::state_manager::WorkflowStateManager;
use crate::ports::storage::StoragePort;
use crate::ports::task_dispatch::{ExecutionResult, TaskDispatchPort};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;

pub struct WorkflowEngine {
    storage: Arc<dyn StoragePort + Send + Sync>,
    task_dispatcher: Arc<dyn TaskDispatchPort + Send + Sync>,
}

#[derive(Default)]
struct ResolvedTaskInputs {
    values: Vec<serde_json::Value>,
    mapping: Vec<TaskInputMapping>,
}

struct VerifierTransition {
    events: Vec<WorkflowInstanceEvent>,
    error_message: Option<String>,
}

/// State machine for a workflow instance.
/// Its main responsibility is: take a persisted WorkflowInstance, read its WorkflowDef,
/// decide which task attempts are runnable, dispatch them through TaskDispatchPort, update task/workflow state,
/// and persist progress back through StoragePort.
impl WorkflowEngine {
    pub fn new(
        storage: Arc<dyn StoragePort + Send + Sync>,
        task_dispatcher: Arc<dyn TaskDispatchPort + Send + Sync>,
    ) -> Self {
        Self {
            storage,
            task_dispatcher,
        }
    }

    /// Returns a lightweight status snapshot of a workflow instance.
    /// Reads the latest persisted state from storage — safe to call at any time,
    /// including while `run_workflow_instance` is executing.
    pub async fn get_workflow_status(
        &self,
        namespace: &Namespace,
        instance_id: &str,
    ) -> anyhow::Result<Option<WorkflowStatusReport>> {
        let Some(instance) = self
            .storage
            .get_workflow_instance(namespace, instance_id)
            .await?
        else {
            return Ok(None);
        };

        let mut tasks: Vec<TaskStatusReport> = instance
            .tasks
            .iter()
            .map(|(task_attempt_id, t)| TaskStatusReport {
                early_exit: t.early_exit,
                task_attempt_id: task_attempt_id.clone(),
                task_def_id: t.task_def_id.clone(),
                status: t.status.clone(),
                satisfaction: t.satisfaction_status.clone(),
                input_mapping: t.input_mapping.clone(),
                generation_index: t.generation_index,
                verifier_metadata: t.verifier_metadata.clone(),
            })
            .collect();

        // Sort for deterministic ordering.
        tasks.sort_by(|a, b| a.task_attempt_id.cmp(&b.task_attempt_id));
        let mut verifier_states: Vec<VerifierStatusReport> = instance
            .verifier_states
            .values()
            .map(|state| VerifierStatusReport {
                verifier_task_id: state.verifier_task_id.clone(),
                rerun_start_task_id: state.rerun_start_task_id.clone(),
                latest_generation: state.latest_generation,
                selected_generation: state.selected_generation,
                status: state.status.clone(),
                feedback_count: state.feedback_history.len(),
                exit_reason: state.exit_reason.clone(),
            })
            .collect();
        verifier_states.sort_by(|a, b| a.verifier_task_id.cmp(&b.verifier_task_id));

        Ok(Some(WorkflowStatusReport {
            instance_id: instance.id,
            workflow_def_id: instance.workflow_def_id,
            status: instance.status,
            tasks,
            verifier_states,
        }))
    }

    pub async fn run_workflow_instance(
        &self,
        namespace: &Namespace,
        workflow_inst_id: String,
    ) -> anyhow::Result<()> {
        let mut workflow_instance = match self
            .storage
            .get_workflow_instance(namespace, &workflow_inst_id)
            .await?
        {
            Some(i) => i,
            None => anyhow::bail!("Workflow instance not found"),
        };

        if matches!(
            workflow_instance.status,
            WorkflowStatus::Paused
                | WorkflowStatus::InputNeeded
                | WorkflowStatus::Completed
                | WorkflowStatus::Failed
        ) {
            return Ok(());
        }

        let workflow_def = match self
            .storage
            .get_workflow_def(namespace, &workflow_instance.workflow_def_id)
            .await?
        {
            Some(d) => d,
            None => anyhow::bail!("Workflow definition not found"),
        };

        let state_manager = WorkflowStateManager::new(Arc::clone(&self.storage));
        workflow_instance = state_manager
            .commit_events_for_instance(
                namespace,
                workflow_instance,
                vec![WorkflowInstanceEvent::WorkflowStatusChanged {
                    status: WorkflowStatus::Running,
                }],
            )
            .await?;

        let loop_slices = self.compute_loop_slices(&workflow_def);

        // Initialize tasks if not already done
        if workflow_instance.tasks.is_empty() {
            let mut events = Vec::with_capacity(workflow_def.tasks.len());
            for task_def in &workflow_def.tasks {
                let task_attempt_id = TaskInstance::make_task_attempt_id(&task_def.id, 1);
                let input_data =
                    initial_task_input_data(&workflow_instance, &workflow_def, task_def);
                events.push(WorkflowInstanceEvent::TaskMaterialized {
                    task_attempt_id,
                    task: TaskInstance {
                        early_exit: false,
                        task_def_id: task_def.id.clone(),
                        status: TaskStatus::Pending,
                        satisfaction_status: TaskSatisfactionStatus::Pending,
                        human_input: None,
                        input_data,
                        input_mapping: vec![],
                        output_data: None,
                        generation_index: 1,
                        verifier_metadata: None,
                    },
                });
            }
            if !events.is_empty() {
                workflow_instance = state_manager
                    .commit_events_for_instance(namespace, workflow_instance, events)
                    .await?;
            }
        }

        // Main Execution Loop
        let mut progress_made = true;
        while progress_made {
            progress_made = false;

            let generation_events = self.materialize_eligible_initial_generation_events(
                &workflow_instance,
                &workflow_def,
                &loop_slices,
            );
            if !generation_events.is_empty() {
                workflow_instance = state_manager
                    .commit_events_for_instance(namespace, workflow_instance, generation_events)
                    .await?;
                progress_made = true;
            }

            let mut tasks_to_run = Vec::new();

            for (task_attempt_id, task_instance) in workflow_instance.tasks.iter() {
                if task_instance.status == TaskStatus::Pending {
                    let task_def = workflow_def
                        .tasks
                        .iter()
                        .find(|t| t.id == task_instance.task_def_id)
                        .unwrap();

                    let can_run = self
                        .resolve_inputs(
                            &workflow_instance,
                            &workflow_def,
                            task_instance,
                            task_def,
                            &loop_slices,
                        )
                        .is_some();

                    if can_run {
                        tasks_to_run.push(task_attempt_id.clone());
                    }
                }
            }
            tasks_to_run.sort();

            for task_attempt_id in tasks_to_run {
                // Earlier results in this pass may have skipped this runnable attempt.
                // TODO: consider other implementation options, e.g. prunning tasks_to_run?
                if workflow_instance.tasks[&task_attempt_id].status != TaskStatus::Pending {
                    continue;
                }
                // A pause may have been committed while this engine pass was
                // resolving runnable work. Stop cleanly instead of turning the
                // next task attempt Running and relying on a version conflict.
                if self
                    .is_workflow_paused(namespace, &workflow_inst_id)
                    .await?
                {
                    return Ok(());
                }

                workflow_instance = state_manager
                    .commit_events_for_instance(
                        namespace,
                        workflow_instance,
                        vec![WorkflowInstanceEvent::TaskStatusChanged {
                            task_attempt_id: task_attempt_id.clone(),
                            status: TaskStatus::Running,
                        }],
                    )
                    .await?;
                progress_made = true;

                let task_instance = workflow_instance
                    .tasks
                    .get(&task_attempt_id)
                    .cloned()
                    .unwrap();

                let task_def = workflow_def
                    .tasks
                    .iter()
                    .find(|t| t.id == task_instance.task_def_id)
                    .unwrap();

                let resolved_inputs = self
                    .resolve_inputs(
                        &workflow_instance,
                        &workflow_def,
                        &task_instance,
                        task_def,
                        &loop_slices,
                    )
                    .unwrap_or_default();

                let inputs = resolved_inputs.values;

                if let Err(error) = validate_inputs(task_def, &inputs) {
                    state_manager
                        .commit_events_for_instance(
                            namespace,
                            workflow_instance,
                            vec![
                                WorkflowInstanceEvent::TaskInputDataSet {
                                    task_attempt_id: task_attempt_id.clone(),
                                    input_data: inputs.clone(),
                                },
                                WorkflowInstanceEvent::TaskInputMappingSet {
                                    task_attempt_id: task_attempt_id.clone(),
                                    input_mapping: resolved_inputs.mapping.clone(),
                                },
                                WorkflowInstanceEvent::TaskStatusChanged {
                                    task_attempt_id: task_attempt_id.clone(),
                                    status: TaskStatus::Failed,
                                },
                                WorkflowInstanceEvent::TaskSatisfactionChanged {
                                    task_attempt_id: task_attempt_id.clone(),
                                    satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                                },
                                WorkflowInstanceEvent::WorkflowStatusChanged {
                                    status: WorkflowStatus::Failed,
                                },
                            ],
                        )
                        .await?;
                    return Err(error);
                }

                let metadata =
                    self.execution_metadata(&workflow_instance, &workflow_def, &task_instance);
                let dispatch = TaskDispatchConstraints {
                    pinned_host_id: workflow_instance.pinned_worker_host.clone(),
                };

                let execution_result =
                    match resolve_task_function_ref(self.storage.as_ref(), namespace, task_def)
                        .await
                    {
                        Ok(resolved_task_def) => {
                            self.task_dispatcher
                                .dispatch_task(
                                    namespace,
                                    &workflow_inst_id,
                                    &resolved_task_def,
                                    &inputs,
                                    &metadata,
                                    &dispatch,
                                )
                                .await
                        }
                        Err(error) => Err(error),
                    };

                match execution_result {
                    Ok(result) => {
                        let output = match result {
                            ExecutionResult::Success(output) => output,
                            // TODO: Consider reducing granularity of those events
                            // i.e. InputNeeded should issue TaskReturnedInputNeeded this should perform all needed internal mutations to task and workflow, statuses etc.
                            ExecutionResult::InputNeeded(input_request) => {
                                self.commit_task_result_events_preserving_pause(
                                    namespace,
                                    &state_manager,
                                    &workflow_inst_id,
                                    workflow_instance,
                                    vec![
                                        WorkflowInstanceEvent::TaskInputDataSet {
                                            task_attempt_id: task_attempt_id.clone(),
                                            input_data: inputs.clone(),
                                        },
                                        WorkflowInstanceEvent::TaskInputMappingSet {
                                            task_attempt_id: task_attempt_id.clone(),
                                            input_mapping: resolved_inputs.mapping.clone(),
                                        },
                                        WorkflowInstanceEvent::TaskStatusChanged {
                                            task_attempt_id: task_attempt_id.clone(),
                                            status: TaskStatus::InputNeeded { input_request },
                                        },
                                        WorkflowInstanceEvent::WorkflowStatusChanged {
                                            status: WorkflowStatus::InputNeeded,
                                        },
                                    ],
                                )
                                .await?;
                                return Ok(());
                            }
                            ExecutionResult::Failure(reason) => {
                                self.commit_task_result_events_preserving_pause(
                                    namespace,
                                    &state_manager,
                                    &workflow_inst_id,
                                    workflow_instance,
                                    vec![
                                        WorkflowInstanceEvent::TaskInputDataSet {
                                            task_attempt_id: task_attempt_id.clone(),
                                            input_data: inputs.clone(),
                                        },
                                        WorkflowInstanceEvent::TaskInputMappingSet {
                                            task_attempt_id: task_attempt_id.clone(),
                                            input_mapping: resolved_inputs.mapping.clone(),
                                        },
                                        WorkflowInstanceEvent::TaskStatusChanged {
                                            task_attempt_id: task_attempt_id.clone(),
                                            status: TaskStatus::Failed,
                                        },
                                        WorkflowInstanceEvent::TaskSatisfactionChanged {
                                            task_attempt_id: task_attempt_id.clone(),
                                            satisfaction_status:
                                                TaskSatisfactionStatus::Unsatisfied,
                                        },
                                        WorkflowInstanceEvent::WorkflowStatusChanged {
                                            status: WorkflowStatus::Failed,
                                        },
                                    ],
                                )
                                .await?;
                                anyhow::bail!("Task execution failed: {}", reason);
                            }
                        };

                        // Validate against output_schema if one is defined; skip for side-effect-only tasks.
                        let output_schema = effective_output_schema(task_def);
                        let schema_ok = match output_schema {
                            Some(schema) => match jsonschema::validator_for(schema) {
                                Ok(validator) => validator.is_valid(&output),
                                Err(_) => false,
                            },
                            None => true,
                        };

                        if schema_ok {
                            let mut events = vec![
                                WorkflowInstanceEvent::TaskInputDataSet {
                                    task_attempt_id: task_attempt_id.clone(),
                                    input_data: inputs.clone(),
                                },
                                WorkflowInstanceEvent::TaskInputMappingSet {
                                    task_attempt_id: task_attempt_id.clone(),
                                    input_mapping: resolved_inputs.mapping.clone(),
                                },
                                WorkflowInstanceEvent::TaskStatusChanged {
                                    task_attempt_id: task_attempt_id.clone(),
                                    status: TaskStatus::Completed,
                                },
                            ];
                            if !self.is_task_in_loop_slice(&loop_slices, &task_def.id) {
                                events.push(WorkflowInstanceEvent::TaskSatisfactionChanged {
                                    task_attempt_id: task_attempt_id.clone(),
                                    satisfaction_status: TaskSatisfactionStatus::Satisfied,
                                });
                            }
                            // Preserve exit output even when no output schema is declared.
                            if output_schema.is_some()
                                || output.get("_workflow_exit_reason").is_some()
                            {
                                events.push(WorkflowInstanceEvent::TaskOutputRecorded {
                                    task_attempt_id: task_attempt_id.clone(),
                                    output_data: Some(output.clone()),
                                });
                            }

                            match crate::core::task::workflow_exit_reason(&output) {
                                Ok(Some(reason)) => {
                                    events.push(WorkflowInstanceEvent::TaskEarlyExitSet {
                                        task_attempt_id: task_attempt_id.clone(),
                                        early_exit: true,
                                    });

                                    let mut pending_ids: Vec<_> = workflow_instance
                                        .tasks
                                        .iter()
                                        .filter(|(_, task)| task.status == TaskStatus::Pending)
                                        .map(|(id, _)| id.clone())
                                        .collect();

                                    // sorting IDs to keep even order deterministic when persisted
                                    pending_ids.sort();

                                    tracing::info!(
                                        workflow_instance_id = %workflow_inst_id,
                                        task_attempt_id = %task_attempt_id,
                                        pending_task_count = pending_ids.len(),
                                        reason = %reason,
                                        "Task provided an exit reason; requesting successful workflow exit"
                                    );

                                    for id in pending_ids {
                                        events.push(WorkflowInstanceEvent::TaskStatusChanged {
                                            task_attempt_id: id.clone(),
                                            status: TaskStatus::Skipped,
                                        });
                                        events.push(
                                            WorkflowInstanceEvent::TaskSatisfactionChanged {
                                                task_attempt_id: id,
                                                satisfaction_status:
                                                    TaskSatisfactionStatus::Unsatisfied,
                                            },
                                        );
                                    }
                                }
                                Ok(None) => {
                                    tracing::info!(
                                        workflow_instance_id = %workflow_inst_id,
                                        task_attempt_id = %task_attempt_id,
                                        "Task provided no exit reason; continuing workflow execution"
                                    );
                                }
                                Err(error) => {
                                    tracing::info!(
                                        workflow_instance_id = %workflow_inst_id,
                                        task_attempt_id = %task_attempt_id,
                                        error = %error,
                                        "Early-exit control validation failed; failing task and workflow"
                                    );

                                    events.extend([
                                        WorkflowInstanceEvent::TaskStatusChanged {
                                            task_attempt_id: task_attempt_id.clone(),
                                            status: TaskStatus::Failed,
                                        },
                                        WorkflowInstanceEvent::TaskSatisfactionChanged {
                                            task_attempt_id: task_attempt_id.clone(),
                                            satisfaction_status:
                                                TaskSatisfactionStatus::Unsatisfied,
                                        },
                                        WorkflowInstanceEvent::WorkflowStatusChanged {
                                            status: WorkflowStatus::Failed,
                                        },
                                    ]);
                                    self.commit_task_result_events_preserving_pause(
                                        namespace,
                                        &state_manager,
                                        &workflow_inst_id,
                                        workflow_instance,
                                        events,
                                    )
                                    .await?;
                                    return Err(error);
                                }
                            }

                            if task_verifier(task_def).is_some() {
                                let verifier_result = match verifier_result_from_output(&output) {
                                    Ok(verifier_result) => verifier_result,
                                    Err(error) => {
                                        events.extend([
                                            WorkflowInstanceEvent::TaskStatusChanged {
                                                task_attempt_id: task_attempt_id.clone(),
                                                status: TaskStatus::Failed,
                                            },
                                            WorkflowInstanceEvent::TaskSatisfactionChanged {
                                                task_attempt_id: task_attempt_id.clone(),
                                                satisfaction_status:
                                                    TaskSatisfactionStatus::Unsatisfied,
                                            },
                                            WorkflowInstanceEvent::TaskVerifierMetadataSet {
                                                task_attempt_id: task_attempt_id.clone(),
                                                verifier_metadata: Some(VerifierAttemptMetadata {
                                                    status: VerifierAttemptStatus::Invalid,
                                                    decision: None,
                                                    feedback: None,
                                                    verifier_output: Some(output.clone()),
                                                    exit_reason: Some(error.to_string()),
                                                }),
                                            },
                                            WorkflowInstanceEvent::WorkflowStatusChanged {
                                                status: WorkflowStatus::Failed,
                                            },
                                        ]);
                                        self.commit_task_result_events_preserving_pause(
                                            namespace,
                                            &state_manager,
                                            &workflow_inst_id,
                                            workflow_instance,
                                            events,
                                        )
                                        .await?;
                                        return Err(error);
                                    }
                                };

                                // Include this result's exit flag and skips when deciding retries.
                                let result_instance =
                                    crate::core::workflow::events::reduce_workflow_instance_events(
                                        Some(workflow_instance.clone()),
                                        &events,
                                    )?;

                                let verifier_transition = self.verifier_result_transition(
                                    &result_instance,
                                    &workflow_def,
                                    &loop_slices,
                                    &task_attempt_id,
                                    &output,
                                    verifier_result,
                                )?;

                                events.extend(verifier_transition.events);

                                workflow_instance = self
                                    .commit_task_result_events_preserving_pause(
                                        namespace,
                                        &state_manager,
                                        &workflow_inst_id,
                                        workflow_instance,
                                        events,
                                    )
                                    .await?;
                                if self
                                    .pause_boundary_reached(
                                        namespace,
                                        &state_manager,
                                        &workflow_instance,
                                        &workflow_def,
                                    )
                                    .await?
                                {
                                    return Ok(());
                                }
                                if let Some(error_message) = verifier_transition.error_message {
                                    anyhow::bail!(error_message);
                                }
                            } else {
                                workflow_instance = self
                                    .commit_task_result_events_preserving_pause(
                                        namespace,
                                        &state_manager,
                                        &workflow_inst_id,
                                        workflow_instance,
                                        events,
                                    )
                                    .await?;
                                if self
                                    .pause_boundary_reached(
                                        namespace,
                                        &state_manager,
                                        &workflow_instance,
                                        &workflow_def,
                                    )
                                    .await?
                                {
                                    return Ok(());
                                }
                            }
                        } else {
                            self.commit_task_result_events_preserving_pause(
                                namespace,
                                &state_manager,
                                &workflow_inst_id,
                                workflow_instance,
                                vec![
                                    WorkflowInstanceEvent::TaskInputDataSet {
                                        task_attempt_id: task_attempt_id.clone(),
                                        input_data: inputs.clone(),
                                    },
                                    WorkflowInstanceEvent::TaskInputMappingSet {
                                        task_attempt_id: task_attempt_id.clone(),
                                        input_mapping: resolved_inputs.mapping.clone(),
                                    },
                                    WorkflowInstanceEvent::TaskStatusChanged {
                                        task_attempt_id: task_attempt_id.clone(),
                                        status: TaskStatus::Failed,
                                    },
                                    WorkflowInstanceEvent::TaskSatisfactionChanged {
                                        task_attempt_id: task_attempt_id.clone(),
                                        satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                                    },
                                    WorkflowInstanceEvent::WorkflowStatusChanged {
                                        status: WorkflowStatus::Failed,
                                    },
                                ],
                            )
                            .await?;
                            anyhow::bail!("Task output failed schema validation");
                        }
                    }
                    Err(e) => {
                        self.commit_task_result_events_preserving_pause(
                            namespace,
                            &state_manager,
                            &workflow_inst_id,
                            workflow_instance,
                            vec![
                                WorkflowInstanceEvent::TaskInputDataSet {
                                    task_attempt_id: task_attempt_id.clone(),
                                    input_data: inputs.clone(),
                                },
                                WorkflowInstanceEvent::TaskInputMappingSet {
                                    task_attempt_id: task_attempt_id.clone(),
                                    input_mapping: resolved_inputs.mapping.clone(),
                                },
                                WorkflowInstanceEvent::TaskStatusChanged {
                                    task_attempt_id: task_attempt_id.clone(),
                                    status: TaskStatus::Failed,
                                },
                                WorkflowInstanceEvent::WorkflowStatusChanged {
                                    status: WorkflowStatus::Failed,
                                },
                            ],
                        )
                        .await?;
                        return Err(e.context("Task execution failed"));
                    }
                }
            }
        }

        if self.workflow_all_completed(&workflow_instance, &workflow_def) {
            state_manager
                .commit_events_for_instance(
                    namespace,
                    workflow_instance,
                    vec![WorkflowInstanceEvent::WorkflowStatusChanged {
                        status: WorkflowStatus::Completed,
                    }],
                )
                .await?;
        }

        Ok(())
    }

    async fn is_workflow_paused(
        &self,
        namespace: &Namespace,
        workflow_inst_id: &str,
    ) -> anyhow::Result<bool> {
        Ok(self
            .storage
            .get_workflow_instance(namespace, workflow_inst_id)
            .await?
            .is_some_and(|instance| instance.status == WorkflowStatus::Paused))
    }

    // If a pause races with an in-flight task, record the task result on top of
    // the latest paused snapshot so pause state is preserved. Optimistic
    // locking still rejects any other stale base snapshot.
    async fn commit_task_result_events_preserving_pause(
        &self,
        namespace: &Namespace,
        state_manager: &WorkflowStateManager,
        workflow_inst_id: &str,
        workflow_instance: WorkflowInstance,
        events: Vec<WorkflowInstanceEvent>,
    ) -> anyhow::Result<WorkflowInstance> {
        let current = self
            .storage
            .get_workflow_instance(namespace, workflow_inst_id)
            .await?;
        let commit_base = match current {
            Some(current) if current.status == WorkflowStatus::Paused => current,
            _ => workflow_instance,
        };

        state_manager
            .commit_events_for_instance(namespace, commit_base, events)
            .await
    }

    async fn pause_boundary_reached(
        &self,
        namespace: &Namespace,
        state_manager: &WorkflowStateManager,
        workflow_instance: &WorkflowInstance,
        workflow_def: &WorkflowDef,
    ) -> anyhow::Result<bool> {
        if workflow_instance.status != WorkflowStatus::Paused {
            return Ok(false);
        }

        if self.workflow_all_completed(workflow_instance, workflow_def) {
            state_manager
                .commit_events_for_instance(
                    namespace,
                    workflow_instance.clone(),
                    vec![WorkflowInstanceEvent::WorkflowStatusChanged {
                        status: WorkflowStatus::Completed,
                    }],
                )
                .await?;
        }

        Ok(true)
    }

    fn workflow_all_completed(
        &self,
        workflow_instance: &WorkflowInstance,
        workflow_def: &WorkflowDef,
    ) -> bool {
        let loop_slices = self.compute_loop_slices(workflow_def);
        workflow_def.tasks.iter().all(|task_def| {
            self.latest_materialized_attempt_id(workflow_instance, &task_def.id)
                .and_then(|id| workflow_instance.tasks.get(&id))
                .is_some_and(|task| {
                    matches!(task.status, TaskStatus::Completed | TaskStatus::Skipped)
                })
        }) && workflow_instance.verifier_states.iter().all(|(id, state)| {
            matches!(
                state.status,
                VerifierStateStatus::Accepted | VerifierStateStatus::ExhaustedAccepted
            ) || (state.status == VerifierStateStatus::Running
                && loop_slices
                    .get(id)
                    .is_some_and(|slice| self.slice_interrupted_by_exit(workflow_instance, slice)))
        })
    }

    fn slice_interrupted_by_exit(&self, instance: &WorkflowInstance, slice: &[String]) -> bool {
        instance.tasks.values().any(|task| {
            slice.contains(&task.task_def_id)
                && (task.status == TaskStatus::Skipped || task.early_exit)
        })
    }

    fn compute_loop_slices(&self, def: &WorkflowDef) -> HashMap<String, Vec<String>> {
        let mut slices = HashMap::new();
        for verifier_task in def
            .tasks
            .iter()
            .filter(|task| task_verifier(task).is_some())
        {
            let verifier = task_verifier(verifier_task).unwrap();
            let rerun_start_task_id = verifier_rerun_start_task_id(verifier, &verifier_task.id);
            let from_start = self.reachable_from(def, &rerun_start_task_id);
            let to_verifier = self.can_reach_target_set(def, &verifier_task.id);
            let slice = def
                .tasks
                .iter()
                .filter_map(|task| {
                    (from_start.contains(&task.id) && to_verifier.contains(&task.id))
                        .then_some(task.id.clone())
                })
                .collect::<Vec<_>>();
            slices.insert(verifier_task.id.clone(), slice);
        }
        slices
    }

    fn reachable_from(&self, def: &WorkflowDef, start: &str) -> HashSet<String> {
        let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
        for binding in &def.data_bindings {
            adjacency
                .entry(binding.source_task_id.clone())
                .or_default()
                .push(binding.target_task_id.clone());
        }
        self.walk_graph(start, &adjacency)
    }

    fn can_reach_target_set(&self, def: &WorkflowDef, target: &str) -> HashSet<String> {
        let mut reverse: HashMap<String, Vec<String>> = HashMap::new();
        for binding in &def.data_bindings {
            reverse
                .entry(binding.target_task_id.clone())
                .or_default()
                .push(binding.source_task_id.clone());
        }
        self.walk_graph(target, &reverse)
    }

    fn walk_graph(&self, start: &str, adjacency: &HashMap<String, Vec<String>>) -> HashSet<String> {
        let mut seen = HashSet::new();
        let mut stack = vec![start.to_string()];
        while let Some(current) = stack.pop() {
            if !seen.insert(current.clone()) {
                continue;
            }
            if let Some(next) = adjacency.get(&current) {
                stack.extend(next.iter().cloned());
            }
        }
        seen
    }

    fn materialize_eligible_initial_generation_events(
        &self,
        instance: &WorkflowInstance,
        def: &WorkflowDef,
        loop_slices: &HashMap<String, Vec<String>>,
    ) -> Vec<WorkflowInstanceEvent> {
        let mut events = Vec::new();
        let mut planned_verifier_states = HashSet::new();
        let mut planned_task_attempts = HashSet::new();

        for (verifier_task_id, slice) in loop_slices {
            if self.slice_interrupted_by_exit(instance, slice)
                || instance.verifier_states.contains_key(verifier_task_id)
                || planned_verifier_states.contains(verifier_task_id)
            {
                continue;
            }

            let Some(verifier_task) = def.tasks.iter().find(|task| task.id == *verifier_task_id)
            else {
                continue;
            };

            let Some(verifier) = task_verifier(verifier_task) else {
                continue;
            };

            let Some(start_task) = def
                .tasks
                .iter()
                .find(|task| task.id == verifier_rerun_start_task_id(verifier, verifier_task_id))
            else {
                continue;
            };

            if !self.logical_task_ready_outside_slice(
                instance,
                def,
                start_task,
                slice.as_slice(),
                loop_slices,
            ) {
                continue;
            }

            events.push(WorkflowInstanceEvent::VerifierStateUpserted {
                verifier_task_id: verifier_task_id.clone(),
                state: VerifierGenerationState {
                    verifier_task_id: verifier_task_id.clone(),
                    rerun_start_task_id: verifier_rerun_start_task_id(verifier, verifier_task_id),
                    latest_generation: 1,
                    selected_generation: None,
                    feedback_history: vec![],
                    status: VerifierStateStatus::Running,
                    exit_reason: None,
                },
            });
            planned_verifier_states.insert(verifier_task_id.clone());
            events.extend(self.materialize_generation_events(
                instance,
                slice,
                1,
                &mut planned_task_attempts,
            ));
        }
        events
    }

    fn logical_task_ready_outside_slice(
        &self,
        instance: &WorkflowInstance,
        def: &WorkflowDef,
        task_def: &crate::core::task::TaskDef,
        slice: &[String],
        loop_slices: &HashMap<String, Vec<String>>,
    ) -> bool {
        def.data_bindings
            .iter()
            .filter(|binding| binding.target_task_id == task_def.id)
            .all(|binding| {
                if slice.contains(&binding.source_task_id) {
                    return true;
                }
                self.resolve_source_attempt_id(instance, loop_slices, None, &binding.source_task_id)
                    .and_then(|source_task_attempt_id| instance.tasks.get(&source_task_attempt_id))
                    .is_some_and(|task| task.status == TaskStatus::Completed)
            })
    }

    fn materialize_generation_events(
        &self,
        workflow_instance: &WorkflowInstance,
        slice: &[String],
        generation_index: u32,
        planned_task_attempts: &mut HashSet<String>,
    ) -> Vec<WorkflowInstanceEvent> {
        if self.slice_interrupted_by_exit(workflow_instance, slice) {
            return vec![];
        }
        let mut events = Vec::new();
        for task_def_id in slice {
            let task_attempt_id = TaskInstance::make_task_attempt_id(task_def_id, generation_index);

            if workflow_instance.tasks.contains_key(&task_attempt_id)
                || !planned_task_attempts.insert(task_attempt_id.clone())
            {
                continue;
            }

            events.push(WorkflowInstanceEvent::TaskMaterialized {
                task_attempt_id,
                task: TaskInstance {
                    early_exit: false,
                    task_def_id: task_def_id.clone(),
                    status: TaskStatus::Pending,
                    satisfaction_status: TaskSatisfactionStatus::Pending,
                    human_input: None,
                    input_data: vec![],
                    input_mapping: vec![],
                    output_data: None,
                    generation_index,
                    verifier_metadata: None,
                },
            });
        }
        events
    }

    // Responsible for evaluating whether or not this task can be executed,
    // and if so, what should be the inputs to this task
    fn resolve_inputs(
        &self,
        workflow_instance: &WorkflowInstance,
        workflow_def: &WorkflowDef,
        task_instance: &TaskInstance,
        task_def: &crate::core::task::TaskDef,
        // list of all verifier slices discovered from the workflow definition
        loop_slices: &HashMap<String, Vec<String>>,
    ) -> Option<ResolvedTaskInputs> {
        // Generation is always present on a TaskInstance. The Option here is for
        // resolve_source_attempt_id callers that may not have a materialized target
        // attempt yet, such as dependency readiness checks.
        let target_attempt_context = Some((
            task_instance.task_def_id.as_str(),
            task_instance.generation_index,
        ));

        let mut inputs = Vec::new();
        let mut mapping = Vec::new();

        let inbound_bindings: Vec<_> = workflow_def
            .data_bindings
            .iter()
            .filter(|binding| binding.target_task_id == task_def.id)
            .collect();

        if inbound_bindings.is_empty() && !task_instance.input_data.is_empty() {
            return Some(ResolvedTaskInputs {
                values: task_instance.input_data.clone(),
                mapping: task_instance.input_mapping.clone(),
            });
        }

        for binding in inbound_bindings {
            let source_task_attempt_id = self.resolve_source_attempt_id(
                workflow_instance,
                &loop_slices,
                target_attempt_context,
                &binding.source_task_id,
            )?;
            let source_task = workflow_instance.tasks.get(&source_task_attempt_id)?;
            if source_task.status != TaskStatus::Completed {
                return None;
            }
            inputs.push(
                source_task
                    .output_data
                    .clone()
                    .unwrap_or(serde_json::Value::Null),
            );
            mapping.push(TaskInputMapping {
                task_id: binding.source_task_id.clone(),
                generation: source_task.generation_index,
            });
        }

        Some(ResolvedTaskInputs {
            values: inputs,
            mapping,
        })
    }

    fn is_task_in_loop_slice(
        &self,
        loop_slices: &HashMap<String, Vec<String>>,
        task_def_id: &str,
    ) -> bool {
        loop_slices
            .iter()
            .any(|(_, slice)| slice.contains(&task_def_id.to_string()))
    }

    // Resolve the concrete source attempt a target attempt should consume.
    // Inside verifier slices, targets consume the latest materialized source
    // attempt and wait until it completes. Outside verifier slices, targets
    // consume the latest completed and satisfied source attempt.
    fn resolve_source_attempt_id(
        &self,
        instance: &WorkflowInstance,
        loop_slices: &HashMap<String, Vec<String>>,
        target_attempt_context: Option<(&str, u32)>,
        source_task_def_id: &str,
    ) -> Option<String> {
        if let Some((target_task_def_id, _generation_index)) = target_attempt_context
            && let Some((_, slice)) = loop_slices
                .iter()
                .find(|(_, slice)| slice.contains(&target_task_def_id.to_string()))
            && slice.contains(&source_task_def_id.to_string())
        {
            return self.latest_materialized_attempt_id(instance, source_task_def_id);
        }

        self.latest_satisfied_attempt_id(instance, source_task_def_id)
    }

    fn latest_materialized_attempt_id(
        &self,
        instance: &WorkflowInstance,
        source_task_def_id: &str,
    ) -> Option<String> {
        instance
            .tasks
            .iter()
            .filter(|(_, task)| task.task_def_id == source_task_def_id)
            .max_by_key(|(_, task)| task.generation_index)
            .map(|(task_attempt_id, _)| task_attempt_id.clone())
    }

    fn latest_satisfied_attempt_id(
        &self,
        instance: &WorkflowInstance,
        source_task_def_id: &str,
    ) -> Option<String> {
        instance
            .tasks
            .iter()
            .filter(|(_, task)| {
                task.task_def_id == source_task_def_id
                    && task.status == TaskStatus::Completed
                    && task.satisfaction_status == TaskSatisfactionStatus::Satisfied
            })
            .max_by_key(|(_, task)| task.generation_index)
            .map(|(task_attempt_id, _)| task_attempt_id.clone())
    }

    fn slice_satisfaction_events(
        &self,
        instance: &WorkflowInstance,
        slice: &[String],
        generation: u32,
        satisfaction: TaskSatisfactionStatus,
    ) -> Vec<WorkflowInstanceEvent> {
        slice
            .iter()
            .filter_map(|task_def_id| {
                let task_attempt_id = TaskInstance::make_task_attempt_id(task_def_id, generation);
                instance.tasks.contains_key(&task_attempt_id).then_some(
                    WorkflowInstanceEvent::TaskSatisfactionChanged {
                        task_attempt_id,
                        satisfaction_status: satisfaction.clone(),
                    },
                )
            })
            .collect()
    }

    fn execution_metadata(
        &self,
        instance: &WorkflowInstance,
        def: &WorkflowDef,
        task_instance: &TaskInstance,
    ) -> ExecutionMetadata {
        let loop_context = self.loop_execution_context(instance, def, task_instance);

        ExecutionMetadata {
            generation_index: task_instance.generation_index,
            loop_context,
            human_input_provided: task_instance
                .human_input
                .as_ref()
                .map(format_human_input_for_execution),
        }
    }

    fn loop_execution_context(
        &self,
        instance: &WorkflowInstance,
        def: &WorkflowDef,
        task_instance: &TaskInstance,
    ) -> Option<LoopExecutionContext> {
        let generation_index = task_instance.generation_index;
        let loop_slices = self.compute_loop_slices(def);
        let Some((verifier_id, _)) = loop_slices
            .iter()
            .find(|(_, slice)| slice.contains(&task_instance.task_def_id))
        else {
            return None;
        };
        let Some(verifier_task) = def.tasks.iter().find(|task| task.id == *verifier_id) else {
            return None;
        };
        let Some(verifier) = task_verifier(verifier_task) else {
            return None;
        };

        let feedback_history = instance
            .verifier_states
            .get(verifier_id)
            .map(|state| {
                state
                    .feedback_history
                    .iter()
                    .map(|entry| LoopFeedbackEntry {
                        generation: entry.generation_index,
                        feedback: entry.feedback.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let previous_output = generation_index
            .checked_sub(1)
            .and_then(|previous_generation| {
                let previous_task_attempt_id = TaskInstance::make_task_attempt_id(
                    &task_instance.task_def_id,
                    previous_generation,
                );
                instance
                    .tasks
                    .get(&previous_task_attempt_id)
                    .and_then(|task| task.output_data.clone())
            });

        Some(LoopExecutionContext {
            generation: generation_index,
            max_iterations: verifier.max_iterations,
            feedback_history,
            previous_output,
        })
    }

    fn verifier_result_transition(
        &self,
        instance: &WorkflowInstance,
        def: &WorkflowDef,
        loop_slices: &HashMap<String, Vec<String>>,
        verifier_task_attempt_id: &str,
        task_output: &serde_json::Value,
        verifier_result: crate::core::verifier::VerifierExecutionResult,
    ) -> anyhow::Result<VerifierTransition> {
        let verifier_task_attempt =
            instance
                .tasks
                .get(verifier_task_attempt_id)
                .ok_or_else(|| {
                    anyhow::anyhow!("verifier task attempt {verifier_task_attempt_id} not found")
                })?;
        let generation = verifier_task_attempt.generation_index;
        let verifier_task_id = verifier_task_attempt.task_def_id.clone();
        let verifier_task = def
            .tasks
            .iter()
            .find(|task| task.id == verifier_task_id)
            .ok_or_else(|| anyhow::anyhow!("verifier task definition missing"))?;
        let verifier = task_verifier(verifier_task)
            .ok_or_else(|| anyhow::anyhow!("task {verifier_task_id} has no verifier config"))?;
        let slice = loop_slices
            .get(&verifier_task_id)
            .cloned()
            .unwrap_or_else(|| vec![verifier_task_id.clone()]);
        let state = instance
            .verifier_states
            .get(&verifier_task_id)
            .ok_or_else(|| anyhow::anyhow!("verifier state {verifier_task_id} missing"))?;
        let mut events = Vec::new();

        match verifier_result.decision {
            VerifierDecision::Complete => {
                events.push(WorkflowInstanceEvent::VerifierStateStatusChanged {
                    verifier_task_id: verifier_task_id.clone(),
                    status: VerifierStateStatus::Accepted,
                    selected_generation: Some(generation),
                    exit_reason: Some("complete".to_string()),
                });
                events.push(WorkflowInstanceEvent::TaskVerifierMetadataSet {
                    task_attempt_id: verifier_task_attempt_id.to_string(),
                    verifier_metadata: Some(VerifierAttemptMetadata {
                        status: VerifierAttemptStatus::Accepted,
                        decision: Some(VerifierDecision::Complete),
                        feedback: verifier_result.feedback,
                        verifier_output: Some(verifier_result.output),
                        exit_reason: Some("complete".to_string()),
                    }),
                });
                events.extend(self.slice_satisfaction_events(
                    instance,
                    &slice,
                    generation,
                    TaskSatisfactionStatus::Satisfied,
                ));
            }
            VerifierDecision::Continue => {
                let feedback = verifier_result.feedback.clone().unwrap_or_default();
                if feedback.trim().is_empty() {
                    events.extend([
                        WorkflowInstanceEvent::WorkflowStatusChanged {
                            status: WorkflowStatus::Failed,
                        },
                        WorkflowInstanceEvent::TaskStatusChanged {
                            task_attempt_id: verifier_task_attempt_id.to_string(),
                            status: TaskStatus::Failed,
                        },
                        WorkflowInstanceEvent::TaskSatisfactionChanged {
                            task_attempt_id: verifier_task_attempt_id.to_string(),
                            satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                        },
                        WorkflowInstanceEvent::TaskVerifierMetadataSet {
                            task_attempt_id: verifier_task_attempt_id.to_string(),
                            verifier_metadata: Some(VerifierAttemptMetadata {
                                status: VerifierAttemptStatus::Invalid,
                                decision: Some(VerifierDecision::Continue),
                                feedback: verifier_result.feedback,
                                verifier_output: Some(verifier_result.output),
                                exit_reason: Some(
                                    "continue decision requires non-empty feedback".to_string(),
                                ),
                            }),
                        },
                    ]);
                    return Ok(VerifierTransition {
                        events,
                        error_message: Some(
                            "Verifier continue decision requires non-empty feedback".to_string(),
                        ),
                    });
                }

                events.push(WorkflowInstanceEvent::VerifierFeedbackRecorded {
                    verifier_task_id: verifier_task_id.clone(),
                    feedback: VerifierFeedbackEntry {
                        generation_index: generation,
                        feedback: feedback.clone(),
                        verifier_output: verifier_result.output.clone(),
                    },
                });

                if self.slice_interrupted_by_exit(instance, &slice) {
                    events.push(WorkflowInstanceEvent::TaskVerifierMetadataSet {
                        task_attempt_id: verifier_task_attempt_id.to_string(),
                        verifier_metadata: Some(VerifierAttemptMetadata {
                            status: VerifierAttemptStatus::Rejected,
                            decision: Some(VerifierDecision::Continue),
                            feedback: Some(feedback),
                            verifier_output: Some(verifier_result.output),
                            exit_reason: Some("slice_interrupted_by_early_exit".to_string()),
                        }),
                    });
                    events.extend(self.slice_satisfaction_events(
                        instance,
                        &slice,
                        generation,
                        TaskSatisfactionStatus::Unsatisfied,
                    ));
                    return Ok(VerifierTransition {
                        events,
                        error_message: None,
                    });
                }

                if generation < verifier.max_iterations {
                    let mut updated_state = state.clone();
                    updated_state.feedback_history.push(VerifierFeedbackEntry {
                        generation_index: generation,
                        feedback: feedback.clone(),
                        verifier_output: verifier_result.output.clone(),
                    });
                    updated_state.latest_generation = generation + 1;
                    updated_state.status = VerifierStateStatus::Running;
                    updated_state.selected_generation = None;
                    updated_state.exit_reason = None;
                    events.push(WorkflowInstanceEvent::VerifierStateUpserted {
                        verifier_task_id: verifier_task_id.clone(),
                        state: updated_state,
                    });
                    events.push(WorkflowInstanceEvent::TaskVerifierMetadataSet {
                        task_attempt_id: verifier_task_attempt_id.to_string(),
                        verifier_metadata: Some(VerifierAttemptMetadata {
                            status: VerifierAttemptStatus::Rejected,
                            decision: Some(VerifierDecision::Continue),
                            feedback: Some(feedback),
                            verifier_output: Some(verifier_result.output),
                            exit_reason: None,
                        }),
                    });
                    events.extend(self.slice_satisfaction_events(
                        instance,
                        &slice,
                        generation,
                        TaskSatisfactionStatus::Unsatisfied,
                    ));
                    events.extend(self.materialize_generation_events(
                        instance,
                        &slice,
                        generation + 1,
                        &mut HashSet::new(),
                    ));
                    return Ok(VerifierTransition {
                        events,
                        error_message: None,
                    });
                }

                if verifier.on_exhausted_continue {
                    if task_output.is_null() && verifier_task.output_schema.is_none() {
                        events.extend([
                            WorkflowInstanceEvent::VerifierStateStatusChanged {
                                verifier_task_id: verifier_task_id.clone(),
                                status: VerifierStateStatus::Failed,
                                selected_generation: state.selected_generation,
                                exit_reason: Some("max_iterations_exhausted".to_string()),
                            },
                            WorkflowInstanceEvent::WorkflowStatusChanged {
                                status: WorkflowStatus::Failed,
                            },
                            WorkflowInstanceEvent::TaskStatusChanged {
                                task_attempt_id: verifier_task_attempt_id.to_string(),
                                status: TaskStatus::Failed,
                            },
                            WorkflowInstanceEvent::TaskSatisfactionChanged {
                                task_attempt_id: verifier_task_attempt_id.to_string(),
                                satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                            },
                            WorkflowInstanceEvent::TaskVerifierMetadataSet {
                                task_attempt_id: verifier_task_attempt_id.to_string(),
                                verifier_metadata: Some(VerifierAttemptMetadata {
                                    status: VerifierAttemptStatus::ExhaustedFailed,
                                    decision: Some(VerifierDecision::Continue),
                                    feedback: Some(feedback),
                                    verifier_output: Some(verifier_result.output),
                                    exit_reason: Some(
                                        "no schema-valid latest generation output".to_string(),
                                    ),
                                }),
                            },
                        ]);
                        return Ok(VerifierTransition {
                            events,
                            error_message: Some(
                                "Verifier exhausted with continue policy but no schema-valid output"
                                    .to_string(),
                            ),
                        });
                    }

                    events.push(WorkflowInstanceEvent::VerifierStateStatusChanged {
                        verifier_task_id: verifier_task_id.clone(),
                        status: VerifierStateStatus::ExhaustedAccepted,
                        selected_generation: Some(generation),
                        exit_reason: Some("max_iterations_exhausted".to_string()),
                    });
                    events.push(WorkflowInstanceEvent::TaskVerifierMetadataSet {
                        task_attempt_id: verifier_task_attempt_id.to_string(),
                        verifier_metadata: Some(VerifierAttemptMetadata {
                            status: VerifierAttemptStatus::ExhaustedAccepted,
                            decision: Some(VerifierDecision::Continue),
                            feedback: Some(feedback),
                            verifier_output: Some(verifier_result.output),
                            exit_reason: Some("max_iterations_exhausted".to_string()),
                        }),
                    });
                    events.extend(self.slice_satisfaction_events(
                        instance,
                        &slice,
                        generation,
                        TaskSatisfactionStatus::Satisfied,
                    ));
                } else {
                    events.extend([
                        WorkflowInstanceEvent::VerifierStateStatusChanged {
                            verifier_task_id: verifier_task_id.clone(),
                            status: VerifierStateStatus::ExhaustedFailed,
                            selected_generation: state.selected_generation,
                            exit_reason: Some("max_iterations_exhausted".to_string()),
                        },
                        WorkflowInstanceEvent::WorkflowStatusChanged {
                            status: WorkflowStatus::Failed,
                        },
                        WorkflowInstanceEvent::TaskStatusChanged {
                            task_attempt_id: verifier_task_attempt_id.to_string(),
                            status: TaskStatus::Failed,
                        },
                        WorkflowInstanceEvent::TaskSatisfactionChanged {
                            task_attempt_id: verifier_task_attempt_id.to_string(),
                            satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                        },
                        WorkflowInstanceEvent::TaskVerifierMetadataSet {
                            task_attempt_id: verifier_task_attempt_id.to_string(),
                            verifier_metadata: Some(VerifierAttemptMetadata {
                                status: VerifierAttemptStatus::ExhaustedFailed,
                                decision: Some(VerifierDecision::Continue),
                                feedback: Some(feedback),
                                verifier_output: Some(verifier_result.output),
                                exit_reason: Some("max_iterations_exhausted".to_string()),
                            }),
                        },
                    ]);
                    events.extend(self.slice_satisfaction_events(
                        instance,
                        &slice,
                        generation,
                        TaskSatisfactionStatus::Unsatisfied,
                    ));
                    return Ok(VerifierTransition {
                        events,
                        error_message: Some("Verifier exhausted iteration budget".to_string()),
                    });
                }
            }
        }

        Ok(VerifierTransition {
            events,
            error_message: None,
        })
    }
}

fn initial_task_input_data(
    workflow_instance: &WorkflowInstance,
    workflow_def: &WorkflowDef,
    task_def: &crate::core::task::TaskDef,
) -> Vec<serde_json::Value> {
    if workflow_instance.trigger_input.is_none() || task_def.input_schemas.is_empty() {
        return vec![];
    }

    let has_inbound_binding = workflow_def
        .data_bindings
        .iter()
        .any(|binding| binding.target_task_id == task_def.id);

    // Return trigger input only if provided task_def has no other dependency
    if has_inbound_binding {
        vec![]
    } else {
        vec![workflow_instance.trigger_input.clone().unwrap()]
    }
}

fn task_verifier(task: &crate::core::task::TaskDef) -> Option<&VerifierControlConfig> {
    task.control.as_ref()?.verifier.as_ref()
}

fn effective_output_schema(
    task: &crate::core::task::TaskDef,
) -> Option<&crate::core::task::JsonSchema> {
    task.output_schema.as_ref()
}

fn validate_inputs(
    task: &crate::core::task::TaskDef,
    inputs: &[serde_json::Value],
) -> anyhow::Result<()> {
    for (index, schema) in task.input_schemas.iter().enumerate() {
        let Some(input) = inputs.get(index) else {
            anyhow::bail!("Task {} missing input at index {}", task.id, index);
        };
        let validator = jsonschema::validator_for(schema).map_err(|error| {
            anyhow::anyhow!(
                "Task {} input schema {} is invalid: {}",
                task.id,
                index,
                error
            )
        })?;
        if !validator.is_valid(input) {
            anyhow::bail!("Task {} input {} failed schema validation", task.id, index);
        }
    }

    Ok(())
}

fn format_human_input_for_execution(input: &serde_json::Value) -> String {
    input.as_str().map(str::to_string).unwrap_or_else(|| {
        serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
    })
}

fn verifier_rerun_start_task_id(
    verifier: &VerifierControlConfig,
    verifier_task_id: &str,
) -> String {
    verifier
        .rerun_from_task_id
        .clone()
        .unwrap_or_else(|| verifier_task_id.to_string())
}

fn verifier_result_from_output(
    output: &serde_json::Value,
) -> anyhow::Result<VerifierExecutionResult> {
    let object = output
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Verifier task output must be an object"))?;
    let decision = match object.get("decision").and_then(|value| value.as_str()) {
        Some("continue") => VerifierDecision::Continue,
        Some("complete") => VerifierDecision::Complete,
        _ => anyhow::bail!("Verifier task decision must be \"continue\" or \"complete\""),
    };
    let feedback = object
        .get("feedback")
        .and_then(|value| value.as_str())
        .map(str::to_string);

    if decision == VerifierDecision::Continue
        && feedback
            .as_deref()
            .is_none_or(|feedback| feedback.trim().is_empty())
    {
        anyhow::bail!("Verifier continue decision requires non-empty feedback");
    }

    Ok(VerifierExecutionResult {
        decision,
        feedback,
        output: output.clone(),
    })
}
