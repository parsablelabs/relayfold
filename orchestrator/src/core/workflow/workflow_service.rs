use crate::core::namespace::Namespace;
use crate::core::task::{TaskDef, TaskInstance, TaskStatus, TaskTypeDef};
use crate::core::verifier::{VerifierControlConfig, verifier_decision_schema};
use crate::core::worker::WorkerHostId;
use crate::core::workflow::events::WorkflowInstanceEvent;
use crate::core::workflow::models::{
    WorkflowDef, WorkflowDefSummary, WorkflowInstance, WorkflowListPage, WorkflowStatus,
};
use crate::core::workflow::state_manager::WorkflowStateManager;
use crate::ports::storage::{
    StoragePort, TaskResult, TaskResultMetadata, WorkflowEventPage, WorkflowEventPageRequest,
    WorkflowInfoCursor, WorkflowInfoPageRequest, WorkflowInstanceFilter, WorkflowTaskResult,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct WorkflowService {
    storage: Arc<dyn StoragePort + Send + Sync>,
    state_manager: WorkflowStateManager,
}

pub const DEFAULT_WORKFLOW_LIST_LIMIT: usize = 50;
pub const MAX_WORKFLOW_LIST_LIMIT: usize = 100;
pub const DEFAULT_WORKFLOW_EVENT_LIST_LIMIT: usize = 50;
pub const MAX_WORKFLOW_EVENT_LIST_LIMIT: usize = 100;

impl WorkflowService {
    pub fn new(storage: Arc<dyn StoragePort + Send + Sync>) -> Self {
        Self {
            state_manager: WorkflowStateManager::new(Arc::clone(&storage)),
            storage,
        }
    }

    pub async fn create_workflow_def(
        &self,
        namespace: &Namespace,
        def: WorkflowDef,
    ) -> anyhow::Result<()> {
        let def = validate_and_normalize_workflow_def(def)?;
        if self
            .storage
            .get_workflow_def(namespace, &def.id)
            .await?
            .is_some()
        {
            let existing_instances = self
                .storage
                .list_workflow_info(
                    Some(namespace),
                    WorkflowInfoPageRequest {
                        limit: 1,
                        cursor: None,
                    },
                    vec![WorkflowInstanceFilter::WorkflowDefId(def.id.clone())],
                )
                .await?;

            if !existing_instances.items.is_empty() {
                anyhow::bail!(
                    "workflow definition {} already has workflow instances and cannot be overwritten; register under a new ID, for example {}_v2",
                    def.id,
                    def.id
                );
            }
        }
        self.storage.save_workflow_def(namespace, def).await?;
        Ok(())
    }

    pub async fn list_workflow_defs(
        &self,
        namespace: &Namespace,
    ) -> anyhow::Result<Vec<WorkflowDefSummary>> {
        Ok(self.storage.list_workflow_def(namespace).await?)
    }

    pub async fn get_workflow_def(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> anyhow::Result<Option<WorkflowDef>> {
        Ok(self.storage.get_workflow_def(namespace, id).await?)
    }

    pub async fn create_workflow_instance_for_def(
        &self,
        namespace: &Namespace,
        workflow_def_id: &str,
        pinned_worker_host: WorkerHostId,
        input: Option<serde_json::Value>,
    ) -> anyhow::Result<String> {
        let Some(_) = self
            .storage
            .get_workflow_def(namespace, workflow_def_id)
            .await?
        else {
            anyhow::bail!("workflow definition {workflow_def_id} not found");
        };

        let instance_id = create_instance_id(workflow_def_id)?;
        let instance = WorkflowInstance {
            id: instance_id.clone(),
            workflow_def_id: workflow_def_id.to_string(),
            version: 0,
            status: WorkflowStatus::Pending,
            trigger_input: input,
            pinned_worker_host: Some(pinned_worker_host),
            tasks: HashMap::new(),
            verifier_states: HashMap::new(),
        };

        self.state_manager
            .commit_events(
                namespace,
                &instance_id,
                vec![WorkflowInstanceEvent::WorkflowCreated { instance }],
            )
            .await?;

        Ok(instance_id)
    }

    pub async fn has_active_workflow_instance_for_def(
        &self,
        namespace: &Namespace,
        workflow_def_id: &str,
    ) -> anyhow::Result<bool> {
        let page = self
            .storage
            .list_workflow_info(
                Some(namespace),
                WorkflowInfoPageRequest {
                    limit: 1,
                    cursor: None,
                },
                vec![
                    WorkflowInstanceFilter::WorkflowDefId(workflow_def_id.to_string()),
                    WorkflowInstanceFilter::Statuses(vec![
                        WorkflowStatus::Pending,
                        WorkflowStatus::Running,
                        WorkflowStatus::Paused,
                        WorkflowStatus::InputNeeded,
                    ]),
                ],
            )
            .await?;

        Ok(!page.items.is_empty())
    }

    pub async fn list_workflows(
        &self,
        namespace: &Namespace,
        status: Option<WorkflowStatus>,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> anyhow::Result<WorkflowListPage> {
        let cursor = cursor
            .map(|cursor| decode_workflow_info_cursor(namespace, cursor))
            .transpose()?;
        let page = self
            .storage
            .list_workflow_info(
                Some(namespace),
                WorkflowInfoPageRequest {
                    limit: clamp_workflow_list_limit(limit),
                    cursor,
                },
                status
                    .map(|status| vec![WorkflowInstanceFilter::Statuses(vec![status])])
                    .unwrap_or_default(),
            )
            .await?;

        Ok(WorkflowListPage {
            workflows: page.items,
            next_cursor: page.next_cursor.map(encode_workflow_info_cursor),
        })
    }

    pub async fn list_workflow_events(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        limit: Option<usize>,
        after_sequence: Option<u64>,
    ) -> anyhow::Result<Option<WorkflowEventPage>> {
        if self
            .storage
            .get_workflow_instance(namespace, workflow_instance_id)
            .await?
            .is_none()
        {
            return Ok(None);
        }

        let events = self
            .storage
            .list_workflow_instance_events(
                namespace,
                workflow_instance_id,
                WorkflowEventPageRequest {
                    limit: limit
                        .unwrap_or(DEFAULT_WORKFLOW_EVENT_LIST_LIMIT)
                        .clamp(1, MAX_WORKFLOW_EVENT_LIST_LIMIT),
                    cursor: after_sequence,
                },
            )
            .await?;

        Ok(Some(events))
    }

    pub async fn pause_workflow(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
    ) -> anyhow::Result<()> {
        let instance = self
            .storage
            .get_workflow_instance(namespace, workflow_instance_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workflow instance {workflow_instance_id} not found"))?;

        match instance.status {
            WorkflowStatus::Pending | WorkflowStatus::Running => {
                self.state_manager
                    .commit_events_for_instance(
                        namespace,
                        instance,
                        vec![WorkflowInstanceEvent::WorkflowStatusChanged {
                            status: WorkflowStatus::Paused,
                        }],
                    )
                    .await?;
                Ok(())
            }
            WorkflowStatus::Paused => Ok(()),
            _ => anyhow::bail!(
                "workflow instance {workflow_instance_id} cannot be paused from its current status"
            ),
        }
    }

    pub async fn resume_workflow(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
    ) -> anyhow::Result<()> {
        let instance = self
            .storage
            .get_workflow_instance(namespace, workflow_instance_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workflow instance {workflow_instance_id} not found"))?;

        if instance.status != WorkflowStatus::Paused {
            anyhow::bail!("workflow instance {workflow_instance_id} is not paused");
        }

        self.state_manager
            .commit_events_for_instance(
                namespace,
                instance,
                vec![WorkflowInstanceEvent::WorkflowStatusChanged {
                    status: WorkflowStatus::Pending,
                }],
            )
            .await?;

        Ok(())
    }

    pub async fn pause_active_workflows(
        &self,
        namespace: &Namespace,
    ) -> anyhow::Result<Vec<String>> {
        let mut cursor: Option<WorkflowInfoCursor> = None;
        let mut paused = Vec::new();

        loop {
            let page = self
                .storage
                .list_workflow_info(
                    Some(namespace),
                    WorkflowInfoPageRequest { limit: 100, cursor },
                    vec![WorkflowInstanceFilter::Statuses(vec![
                        WorkflowStatus::Pending,
                        WorkflowStatus::Running,
                    ])],
                )
                .await?;

            for info in &page.items {
                self.pause_workflow(namespace, &info.id).await?;
                paused.push(info.id.clone());
            }

            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        Ok(paused)
    }

    pub async fn resume_paused_workflows(
        &self,
        namespace: &Namespace,
    ) -> anyhow::Result<Vec<String>> {
        let mut cursor: Option<WorkflowInfoCursor> = None;
        let mut resumed = Vec::new();

        loop {
            let page = self
                .storage
                .list_workflow_info(
                    Some(namespace),
                    WorkflowInfoPageRequest { limit: 100, cursor },
                    vec![WorkflowInstanceFilter::Statuses(vec![
                        WorkflowStatus::Paused,
                    ])],
                )
                .await?;

            for info in &page.items {
                self.resume_workflow(namespace, &info.id).await?;
                resumed.push(info.id.clone());
            }

            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        Ok(resumed)
    }

    pub async fn submit_human_input(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        task_id: &str,
        submitted_input: serde_json::Value,
    ) -> anyhow::Result<String> {
        let instance = self
            .storage
            .get_workflow_instance(namespace, workflow_instance_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workflow instance {workflow_instance_id} not found"))?;

        if instance.status != WorkflowStatus::InputNeeded {
            anyhow::bail!("workflow instance {workflow_instance_id} is not waiting for input");
        }

        let task_attempt_id = resolve_task_attempt_id(&instance, task_id, None)?;
        let task = instance
            .tasks
            .get(&task_attempt_id)
            .ok_or_else(|| anyhow::anyhow!("task {task_id} not found"))?;

        if !matches!(task.status, TaskStatus::InputNeeded { .. }) {
            anyhow::bail!("task {task_id} is not waiting for input");
        }

        let task_def_id = task.task_def_id.clone();
        let next_generation = task.generation_index + 1;

        let Some(workflow_def) = self
            .storage
            .get_workflow_def(namespace, &instance.workflow_def_id)
            .await?
        else {
            anyhow::bail!("workflow definition {} not found", instance.workflow_def_id);
        };

        let task_def = workflow_def
            .tasks
            .iter()
            .find(|task_def| task_def.id == task_def_id)
            .ok_or_else(|| anyhow::anyhow!("task definition {task_def_id} not found"))?;
        if !matches!(task_def.kind, TaskTypeDef::Agent { .. }) {
            anyhow::bail!("task {task_id} is not an Agent task");
        }

        let next_task_attempt_id =
            TaskInstance::make_task_attempt_id(&task_def_id, next_generation);

        // This could happen if request was submitted more than once
        if instance.tasks.contains_key(&next_task_attempt_id) {
            anyhow::bail!("task {task_id} already has a materialized continuation attempt");
        }

        self.state_manager
            .commit_events_for_instance(
                namespace,
                instance,
                vec![WorkflowInstanceEvent::HumanInputSubmitted {
                    task_attempt_id: task_attempt_id.clone(),
                    continuation_task_attempt_id: next_task_attempt_id.clone(),
                    submitted_input,
                }],
            )
            .await?;

        Ok(next_task_attempt_id)
    }

    pub async fn retry_task(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        task_id: &str,
    ) -> anyhow::Result<String> {
        let (instance, task_attempt_id) = self
            .load_retryable_task_instance(namespace, workflow_instance_id, task_id)
            .await?;

        self.state_manager
            .commit_events_for_instance(
                namespace,
                instance,
                vec![WorkflowInstanceEvent::TaskRetryRequested {
                    task_attempt_id: task_attempt_id.clone(),
                }],
            )
            .await?;

        Ok(task_attempt_id)
    }

    pub async fn load_retryable_task_pin(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        task_id: &str,
    ) -> anyhow::Result<Option<WorkerHostId>> {
        let (instance, _) = self
            .load_retryable_task_instance(namespace, workflow_instance_id, task_id)
            .await?;
        Ok(instance.pinned_worker_host)
    }

    pub async fn force_retry_task(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        task_id: &str,
        target_host_id: WorkerHostId,
    ) -> anyhow::Result<String> {
        let (instance, task_attempt_id) = self
            .load_retryable_task_instance(namespace, workflow_instance_id, task_id)
            .await?;
        let previous_host_id = instance.pinned_worker_host.clone();
        let local_context_may_be_lost = previous_host_id.as_ref() != Some(&target_host_id);

        self.state_manager
            .commit_events_for_instance(
                namespace,
                instance,
                vec![WorkflowInstanceEvent::TaskForceRetryRequested {
                    task_attempt_id: task_attempt_id.clone(),
                    previous_host_id,
                    target_host_id,
                    local_context_may_be_lost,
                }],
            )
            .await?;

        Ok(task_attempt_id)
    }

    pub async fn get_task_result(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        requested_task_id: &str,
    ) -> anyhow::Result<TaskResult> {
        self.get_task_result_for_generation(
            namespace,
            workflow_instance_id,
            requested_task_id,
            None,
        )
        .await
    }

    pub async fn list_task_results(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
    ) -> anyhow::Result<Vec<WorkflowTaskResult>> {
        let instance = self
            .storage
            .get_workflow_instance(namespace, workflow_instance_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workflow instance {workflow_instance_id} not found"))?;

        let mut tasks: Vec<WorkflowTaskResult> = instance
            .tasks
            .iter()
            .map(|(task_attempt_id, task)| WorkflowTaskResult {
                task_attempt_id: task_attempt_id.clone(),
                result: task_result_for_instance(
                    task_attempt_id,
                    task,
                    should_include_task_result_metadata(
                        task_attempt_id,
                        task_attempt_id,
                        task,
                        None,
                    ),
                ),
            })
            .collect();
        tasks.sort_by(|left, right| left.task_attempt_id.cmp(&right.task_attempt_id));

        Ok(tasks)
    }

    pub async fn get_task_result_for_generation(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        requested_task_id: &str,
        generation: Option<u32>,
    ) -> anyhow::Result<TaskResult> {
        let instance = self
            .storage
            .get_workflow_instance(namespace, workflow_instance_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workflow instance {workflow_instance_id} not found"))?;

        let task_attempt_id = resolve_task_attempt_id(&instance, requested_task_id, generation)?;

        let task = instance
            .tasks
            .get(&task_attempt_id)
            .ok_or_else(|| anyhow::anyhow!("task {requested_task_id} not found"))?;

        Ok(task_result_for_instance(
            &task_attempt_id,
            task,
            should_include_task_result_metadata(
                requested_task_id,
                &task_attempt_id,
                task,
                generation,
            ),
        ))
    }

    async fn load_retryable_task_instance(
        &self,
        namespace: &Namespace,
        workflow_instance_id: &str,
        task_id: &str,
    ) -> anyhow::Result<(WorkflowInstance, String)> {
        let instance = self
            .storage
            .get_workflow_instance(namespace, workflow_instance_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("workflow instance {workflow_instance_id} not found"))?;

        if instance.status != WorkflowStatus::Failed {
            anyhow::bail!("workflow instance {workflow_instance_id} is not failed");
        }

        let task_attempt_id = resolve_task_attempt_id(&instance, task_id, None)?;
        let task = instance
            .tasks
            .get(&task_attempt_id)
            .ok_or_else(|| anyhow::anyhow!("task {task_id} not found"))?;

        if task.status != TaskStatus::Failed {
            anyhow::bail!("task {task_id} is not failed");
        }

        Ok((instance, task_attempt_id))
    }
}

fn resolve_task_attempt_id(
    instance: &WorkflowInstance,
    requested_task_id: &str,
    requested_generation: Option<u32>,
) -> anyhow::Result<String> {
    let normalized_task_id = requested_task_id.to_ascii_lowercase();

    if let Some(generation) = requested_generation {
        if generation == 0 {
            anyhow::bail!("generation must be positive");
        }
        return Ok(TaskInstance::make_task_attempt_id(
            &normalized_task_id,
            generation,
        ));
    }

    if instance.tasks.contains_key(requested_task_id) {
        return Ok(requested_task_id.to_string());
    }

    if normalized_task_id != requested_task_id && instance.tasks.contains_key(&normalized_task_id) {
        return Ok(normalized_task_id);
    }

    if let Some((task_attempt_id, _)) = instance
        .tasks
        .iter()
        .filter(|(_, task)| task.task_def_id == normalized_task_id)
        .max_by_key(|(_, task)| task.generation_index)
    {
        return Ok(task_attempt_id.clone());
    }

    anyhow::bail!("task {requested_task_id} not found")
}

fn should_include_task_result_metadata(
    requested_task_id: &str,
    task_attempt_id: &str,
    task: &TaskInstance,
    requested_generation: Option<u32>,
) -> bool {
    requested_generation.is_some()
        || requested_task_id != task_attempt_id
        || task.generation_index > 0
        || task.verifier_metadata.is_some()
}

fn task_result_for_instance(
    task_attempt_id: &str,
    task: &TaskInstance,
    include_metadata: bool,
) -> TaskResult {
    let metadata = include_metadata.then(|| TaskResultMetadata {
        task_def_id: task.task_def_id.clone(),
        task_attempt_id: task_attempt_id.to_string(),
        satisfaction: task.satisfaction_status.clone(),
        input_mapping: task.input_mapping.clone(),
        generation_index: task.generation_index,
        verifier_metadata: task.verifier_metadata.clone(),
    });

    match &task.status {
        TaskStatus::Completed => TaskResult::Success {
            input: task.input_data.clone(),
            output: task.output_data.clone().unwrap_or(serde_json::Value::Null),
            metadata,
        },
        TaskStatus::Failed => TaskResult::Failure {
            input: task.input_data.clone(),
            error_message: "task failed".to_string(),
            metadata,
        },
        TaskStatus::Pending => TaskResult::Pending {
            input: task.input_data.clone(),
            metadata,
        },
        TaskStatus::Running => TaskResult::Running {
            input: task.input_data.clone(),
            metadata,
        },
        TaskStatus::InputNeeded { input_request } => TaskResult::InputNeeded {
            input: task.input_data.clone(),
            input_request: input_request.clone(),
            metadata,
        },
    }
}

fn create_instance_id(workflow_def_id: &str) -> anyhow::Result<String> {
    let timestamp_nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(format!("{workflow_def_id}-{timestamp_nanos}"))
}

fn clamp_workflow_list_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(DEFAULT_WORKFLOW_LIST_LIMIT)
        .clamp(1, MAX_WORKFLOW_LIST_LIMIT)
}

fn encode_workflow_info_cursor(cursor: WorkflowInfoCursor) -> String {
    format!(
        "{}:{}:{}",
        cursor.namespace, cursor.modified_at_epoch_ms, cursor.workflow_instance_id
    )
}

fn decode_workflow_info_cursor(
    namespace: &Namespace,
    cursor: &str,
) -> anyhow::Result<WorkflowInfoCursor> {
    let mut parts = cursor.splitn(3, ':');
    let (Some(cursor_namespace), Some(modified_at_epoch_ms), Some(workflow_instance_id)) =
        (parts.next(), parts.next(), parts.next())
    else {
        anyhow::bail!("invalid workflow list cursor");
    };
    if cursor_namespace != namespace.as_str() || workflow_instance_id.is_empty() {
        anyhow::bail!("invalid workflow list cursor");
    }

    Ok(WorkflowInfoCursor {
        namespace: namespace.clone(),
        modified_at_epoch_ms: modified_at_epoch_ms
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid workflow list cursor"))?,
        workflow_instance_id: workflow_instance_id.to_string(),
    })
}

// Returns a map of verifier task ID to the set of task IDs that are in its loop slice.
fn compute_loop_slices(def: &WorkflowDef) -> HashMap<String, Vec<String>> {
    let mut slices = HashMap::new();

    for verifier_task in def
        .tasks
        .iter()
        .filter(|task| get_task_verifier_config(task).is_some())
    {
        let verifier_config = get_task_verifier_config(verifier_task).unwrap();
        let rerun_start_task_id = verifier_rerun_start_task_id(verifier_config, &verifier_task.id);
        let from_start = reachable_from(def, &rerun_start_task_id);
        let to_verifier = can_reach_target_set(def, &verifier_task.id);
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

fn reachable_from(def: &WorkflowDef, start: &str) -> HashSet<String> {
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for binding in &def.data_bindings {
        adjacency
            .entry(binding.source_task_id.clone())
            .or_default()
            .push(binding.target_task_id.clone());
    }
    walk_graph(start, &adjacency)
}

fn can_reach_target_set(def: &WorkflowDef, target: &str) -> HashSet<String> {
    let mut reverse: HashMap<String, Vec<String>> = HashMap::new();
    for binding in &def.data_bindings {
        reverse
            .entry(binding.target_task_id.clone())
            .or_default()
            .push(binding.source_task_id.clone());
    }
    walk_graph(target, &reverse)
}

fn walk_graph(start: &str, adjacency: &HashMap<String, Vec<String>>) -> HashSet<String> {
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

fn get_task_verifier_config(task: &TaskDef) -> Option<&VerifierControlConfig> {
    task.control.as_ref()?.verifier.as_ref()
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

fn validate_and_normalize_workflow_def(mut def: WorkflowDef) -> anyhow::Result<WorkflowDef> {
    validate_identifier("workflow", &def.id)?;
    def.id = def.id.to_ascii_lowercase();

    let mut original_to_normalized = HashMap::new();
    let mut normalized_task_ids = HashSet::new();

    for task in &mut def.tasks {
        validate_identifier("task", &task.id)?;
        let normalized = task.id.to_ascii_lowercase();
        if !normalized_task_ids.insert(normalized.clone()) {
            anyhow::bail!("duplicate task id after normalization: {normalized}");
        }
        original_to_normalized.insert(task.id.clone(), normalized.clone());
        task.id = normalized;
        if let Some(workspace) = task.workspace.as_mut() {
            validate_identifier("workspace group", &workspace.group_name)?;
            workspace.group_name = workspace.group_name.to_ascii_lowercase();
        }
    }

    for binding in &mut def.data_bindings {
        binding.source_task_id = normalize_task_reference(
            &original_to_normalized,
            &binding.source_task_id,
            "data binding source",
        )?;
        binding.target_task_id = normalize_task_reference(
            &original_to_normalized,
            &binding.target_task_id,
            "data binding target",
        )?;
    }

    for task in &mut def.tasks {
        if get_task_verifier_config(task).is_some() && task.output_schema.is_some() {
            anyhow::bail!(
                "task {} declares control.verifier and must not declare output_schema",
                task.id
            );
        }
        if let Some(verifier) = task
            .control
            .as_mut()
            .and_then(|control| control.verifier.as_mut())
        {
            if verifier.max_iterations == 0 {
                anyhow::bail!("task {} verifier max_iterations must be positive", task.id);
            }
            if let Some(rerun_from_task_id) = verifier.rerun_from_task_id.as_mut() {
                *rerun_from_task_id = normalize_task_reference(
                    &original_to_normalized,
                    rerun_from_task_id,
                    "verifier rerun_from_task_id",
                )?;
            }
            task.output_schema = Some(verifier_decision_schema());
        }
    }

    if has_data_binding_cycle(&def) {
        anyhow::bail!("workflow data bindings contain a cycle");
    }

    for task in &def.tasks {
        if let Some(verifier) = get_task_verifier_config(task) {
            let Some(rerun_from_task_id) = &verifier.rerun_from_task_id else {
                continue;
            };
            if rerun_from_task_id != &task.id
                && !is_upstream_ancestor(&def, rerun_from_task_id, &task.id)
            {
                anyhow::bail!(
                    "task {} verifier rerun task {} is not an upstream ancestor",
                    task.id,
                    rerun_from_task_id
                );
            }
        }
    }

    validate_non_overlapping_verifier_slices(&def)?;

    Ok(def)
}

fn validate_identifier(kind: &str, id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!(
            "{kind} id {id:?} must contain only ASCII alphanumeric characters, '-' or '_'"
        );
    }
    Ok(())
}

fn normalize_task_reference(
    original_to_normalized: &HashMap<String, String>,
    id: &str,
    field: &str,
) -> anyhow::Result<String> {
    validate_identifier(field, id)?;
    original_to_normalized
        .get(id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{field} references unknown task id {id}"))
}

fn has_data_binding_cycle(def: &WorkflowDef) -> bool {
    let mut in_degree: HashMap<String, usize> = def
        .tasks
        .iter()
        .map(|task| (task.id.clone(), 0usize))
        .collect();
    let mut adjacency: HashMap<String, Vec<String>> = def
        .tasks
        .iter()
        .map(|task| (task.id.clone(), Vec::new()))
        .collect();

    for binding in &def.data_bindings {
        *in_degree.entry(binding.target_task_id.clone()).or_insert(0) += 1;
        adjacency
            .entry(binding.source_task_id.clone())
            .or_default()
            .push(binding.target_task_id.clone());
    }

    let mut queue = in_degree
        .iter()
        .filter_map(|(node, degree)| (*degree == 0).then_some(node.clone()))
        .collect::<VecDeque<_>>();
    let mut visited = 0usize;

    while let Some(node) = queue.pop_front() {
        visited += 1;
        if let Some(neighbors) = adjacency.get(&node) {
            for neighbor in neighbors {
                if let Some(degree) = in_degree.get_mut(neighbor) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(neighbor.clone());
                    }
                }
            }
        }
    }

    visited != def.tasks.len()
}

fn is_upstream_ancestor(def: &WorkflowDef, ancestor: &str, task_id: &str) -> bool {
    let mut reverse: HashMap<String, Vec<String>> = HashMap::new();
    for binding in &def.data_bindings {
        reverse
            .entry(binding.target_task_id.clone())
            .or_default()
            .push(binding.source_task_id.clone());
    }

    let mut stack = vec![task_id.to_string()];
    let mut seen = HashSet::new();
    while let Some(current) = stack.pop() {
        if !seen.insert(current.clone()) {
            continue;
        }
        if let Some(upstream) = reverse.get(&current) {
            for source in upstream {
                if source == ancestor {
                    return true;
                }
                stack.push(source.clone());
            }
        }
    }

    false
}

fn validate_non_overlapping_verifier_slices(def: &WorkflowDef) -> anyhow::Result<()> {
    let loop_slices = compute_loop_slices(def);
    let mut owners: HashMap<String, String> = HashMap::new();

    for (verifier_id, slice) in loop_slices {
        for task_id in slice {
            if let Some(existing_verifier_id) = owners.insert(task_id.clone(), verifier_id.clone())
            {
                anyhow::bail!(
                    "verifier loop slices overlap on task {task_id}: {existing_verifier_id} and {verifier_id}"
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::memory_storage::MemoryStorage;
    use crate::core::function::models::FunctionTaskDef;
    use crate::core::task::{TaskSatisfactionStatus, TaskTypeDef};
    use serde_json::json;

    #[test]
    fn workflow_cursor_rejects_a_different_namespace() {
        let namespace = crate::core::namespace::test_namespace();
        let other_namespace = Namespace::new("123e4567-e89b-12d3-a456-426614174000").unwrap();
        let cursor = encode_workflow_info_cursor(WorkflowInfoCursor {
            namespace: other_namespace,
            modified_at_epoch_ms: 123,
            workflow_instance_id: "workflow-1".to_string(),
        });

        assert!(decode_workflow_info_cursor(&namespace, &cursor).is_err());
    }

    fn workflow_def(id: &str) -> WorkflowDef {
        workflow_def_with_task(id, "taska")
    }

    fn workflow_def_with_task(id: &str, task_id: &str) -> WorkflowDef {
        WorkflowDef {
            id: id.to_string(),
            description: String::new(),
            tasks: vec![TaskDef {
                id: task_id.to_string(),
                kind: TaskTypeDef::Function(FunctionTaskDef::Inline {
                    dependencies: vec![],
                    code: "export default async function run() { return {}; }".to_string(),
                }),
                control: None,
                timeout_secs: None,
                input_schemas: vec![],
                output_schema: Some(json!({ "type": "object" })),
                workspace: None,
                required_credentials: vec![],
            }],
            data_bindings: vec![],
        }
    }

    fn agent_task_def(task_id: &str) -> TaskDef {
        TaskDef {
            id: task_id.to_string(),
            kind: TaskTypeDef::Agent {
                model_id: "test/model".to_string(),
                provider_url: "".to_string(),
                prompt: "continue".to_string(),
                tools: vec![],
                skills: vec![],
                ask: true,
                schema_failure_retry_times: 0.into(),
                reuse_session: true,
            },
            control: None,
            timeout_secs: None,
            input_schemas: vec![],
            output_schema: Some(json!({ "type": "object" })),
            workspace: None,
            required_credentials: vec![],
        }
    }

    #[tokio::test]
    async fn create_workflow_def_allows_overwrite_before_instances_exist() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());

        service
            .create_workflow_def(
                &crate::core::namespace::test_namespace(),
                workflow_def_with_task("workflow1", "taska"),
            )
            .await
            .unwrap();
        service
            .create_workflow_def(
                &crate::core::namespace::test_namespace(),
                workflow_def_with_task("workflow1", "taskb"),
            )
            .await
            .unwrap();

        let stored = storage
            .get_workflow_def(&crate::core::namespace::test_namespace(), "workflow1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.tasks[0].id, "taskb");
    }

    #[tokio::test]
    async fn create_workflow_def_rejects_overwrite_after_instance_exists() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());

        service
            .create_workflow_def(
                &crate::core::namespace::test_namespace(),
                workflow_def("workflow1"),
            )
            .await
            .unwrap();
        service
            .create_workflow_instance_for_def(
                &crate::core::namespace::test_namespace(),
                "workflow1",
                WorkerHostId::new("test-host"),
                None,
            )
            .await
            .unwrap();

        let error = service
            .create_workflow_def(
                &crate::core::namespace::test_namespace(),
                workflow_def_with_task("workflow1", "taskb"),
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "workflow definition workflow1 already has workflow instances and cannot be overwritten; register under a new ID, for example workflow1_v2"
        );
        let stored = storage
            .get_workflow_def(&crate::core::namespace::test_namespace(), "workflow1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.tasks[0].id, "taska");
    }

    #[tokio::test]
    async fn create_workflow_def_rejects_overwrite_when_instance_exists_in_any_state() {
        for (index, status) in [
            WorkflowStatus::Pending,
            WorkflowStatus::Running,
            WorkflowStatus::Paused,
            WorkflowStatus::InputNeeded,
            WorkflowStatus::Completed,
            WorkflowStatus::Failed,
        ]
        .into_iter()
        .enumerate()
        {
            let storage = Arc::new(MemoryStorage::new());
            let service = WorkflowService::new(storage.clone());

            service
                .create_workflow_def(
                    &crate::core::namespace::test_namespace(),
                    workflow_def("workflow1"),
                )
                .await
                .unwrap();
            storage
                .save_workflow_instance(
                    &crate::core::namespace::test_namespace(),
                    0,
                    vec![],
                    WorkflowInstance {
                        id: format!("workflow-{index}"),
                        workflow_def_id: "workflow1".to_string(),
                        version: 0,
                        status,
                        trigger_input: None,
                        pinned_worker_host: None,
                        tasks: HashMap::new(),
                        verifier_states: HashMap::new(),
                    },
                )
                .await
                .unwrap();

            let error = service
                .create_workflow_def(
                    &crate::core::namespace::test_namespace(),
                    workflow_def_with_task("workflow1", "taskb"),
                )
                .await
                .unwrap_err();

            assert!(error.to_string().contains("cannot be overwritten"));
            let stored = storage
                .get_workflow_def(&crate::core::namespace::test_namespace(), "workflow1")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.tasks[0].id, "taska");
        }
    }

    #[tokio::test]
    async fn pause_and_resume_workflow_record_status_events() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                WorkflowInstance {
                    id: "active-workflow".to_string(),
                    workflow_def_id: "workflow1".to_string(),
                    version: 0,
                    status: WorkflowStatus::Running,
                    trigger_input: None,
                    pinned_worker_host: Some(WorkerHostId::new("test-host")),
                    tasks: HashMap::new(),
                    verifier_states: HashMap::new(),
                },
            )
            .await
            .unwrap();

        service
            .pause_workflow(&crate::core::namespace::test_namespace(), "active-workflow")
            .await
            .unwrap();
        let paused = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "active-workflow")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(paused.status, WorkflowStatus::Paused);
        assert_eq!(
            paused.pinned_worker_host,
            Some(WorkerHostId::new("test-host"))
        );

        service
            .resume_workflow(&crate::core::namespace::test_namespace(), "active-workflow")
            .await
            .unwrap();
        let resumed = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "active-workflow")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resumed.status, WorkflowStatus::Pending);

        let events = storage
            .list_workflow_instance_events(
                &crate::core::namespace::test_namespace(),
                "active-workflow",
                WorkflowEventPageRequest {
                    limit: 100,
                    cursor: None,
                },
            )
            .await
            .unwrap()
            .items;
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0].event,
            WorkflowInstanceEvent::WorkflowStatusChanged {
                status: WorkflowStatus::Paused
            }
        ));
        assert!(matches!(
            &events[1].event,
            WorkflowInstanceEvent::WorkflowStatusChanged {
                status: WorkflowStatus::Pending
            }
        ));
    }

    #[tokio::test]
    async fn pause_active_and_resume_paused_workflows_apply_bulk_filters() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        for (id, status) in [
            ("pending-workflow", WorkflowStatus::Pending),
            ("running-workflow", WorkflowStatus::Running),
            ("paused-workflow", WorkflowStatus::Paused),
            ("input-needed-workflow", WorkflowStatus::InputNeeded),
            ("completed-workflow", WorkflowStatus::Completed),
            ("failed-workflow", WorkflowStatus::Failed),
        ] {
            storage
                .save_workflow_instance(
                    &crate::core::namespace::test_namespace(),
                    0,
                    vec![],
                    WorkflowInstance {
                        id: id.to_string(),
                        workflow_def_id: "workflow1".to_string(),
                        version: 0,
                        status,
                        trigger_input: None,
                        pinned_worker_host: None,
                        tasks: HashMap::new(),
                        verifier_states: HashMap::new(),
                    },
                )
                .await
                .unwrap();
        }

        let mut paused = service
            .pause_active_workflows(&crate::core::namespace::test_namespace())
            .await
            .unwrap();
        paused.sort();
        assert_eq!(
            paused,
            vec![
                "pending-workflow".to_string(),
                "running-workflow".to_string(),
            ]
        );

        let mut resumed = service
            .resume_paused_workflows(&crate::core::namespace::test_namespace())
            .await
            .unwrap();
        resumed.sort();
        assert_eq!(
            resumed,
            vec![
                "paused-workflow".to_string(),
                "pending-workflow".to_string(),
                "running-workflow".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn create_workflow_instance_for_def_persists_pending_instance() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        service
            .create_workflow_def(
                &crate::core::namespace::test_namespace(),
                workflow_def("workflow1"),
            )
            .await
            .unwrap();

        let instance_id = service
            .create_workflow_instance_for_def(
                &crate::core::namespace::test_namespace(),
                "workflow1",
                WorkerHostId::new("test-host"),
                None,
            )
            .await
            .unwrap();

        let instance = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(instance.workflow_def_id, "workflow1");
        assert_eq!(instance.status, WorkflowStatus::Pending);
        assert_eq!(
            instance.pinned_worker_host,
            Some(WorkerHostId::new("test-host"))
        );
        assert!(instance.tasks.is_empty());
        assert!(instance.verifier_states.is_empty());

        let events = service
            .list_workflow_events(
                &crate::core::namespace::test_namespace(),
                &instance_id,
                None,
                None,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(events.items.len(), 1);
        assert!(events.items[0].created_time > 0);
    }

    #[tokio::test]
    async fn create_workflow_instance_for_def_persists_trigger_input() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        let mut def = workflow_def("workflow1");
        def.tasks[0].input_schemas = vec![json!({
            "type": "object",
            "required": ["repository"],
            "properties": {
                "repository": { "type": "string" }
            }
        })];
        service
            .create_workflow_def(&crate::core::namespace::test_namespace(), def)
            .await
            .unwrap();

        let input = json!({ "repository": "parsablelabs/relayfold" });
        let instance_id = service
            .create_workflow_instance_for_def(
                &crate::core::namespace::test_namespace(),
                "workflow1",
                WorkerHostId::new("test-host"),
                Some(input.clone()),
            )
            .await
            .unwrap();

        let instance = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), &instance_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(instance.trigger_input, Some(input));
        assert!(instance.tasks.is_empty());
    }

    #[tokio::test]
    async fn create_workflow_instance_for_def_rejects_unknown_definition() {
        let service = WorkflowService::new(Arc::new(MemoryStorage::new()));

        let error = service
            .create_workflow_instance_for_def(
                &crate::core::namespace::test_namespace(),
                "missing",
                WorkerHostId::new("test-host"),
                None,
            )
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("workflow definition missing not found")
        );
    }

    #[tokio::test]
    async fn active_workflow_instance_lookup_matches_only_nonterminal_instances_for_definition() {
        let namespace = crate::core::namespace::test_namespace();

        for status in [
            WorkflowStatus::Pending,
            WorkflowStatus::Running,
            WorkflowStatus::Paused,
            WorkflowStatus::InputNeeded,
        ] {
            let storage = Arc::new(MemoryStorage::new());
            let service = WorkflowService::new(storage.clone());
            storage
                .save_workflow_instance(
                    &namespace,
                    0,
                    vec![],
                    WorkflowInstance {
                        id: format!("{status:?}-workflow"),
                        workflow_def_id: "scheduled-workflow".to_string(),
                        version: 0,
                        status,
                        trigger_input: None,
                        pinned_worker_host: None,
                        tasks: HashMap::new(),
                        verifier_states: HashMap::new(),
                    },
                )
                .await
                .unwrap();

            assert!(
                service
                    .has_active_workflow_instance_for_def(&namespace, "scheduled-workflow")
                    .await
                    .unwrap()
            );
            assert!(
                !service
                    .has_active_workflow_instance_for_def(&namespace, "other-workflow")
                    .await
                    .unwrap()
            );
        }

        for status in [WorkflowStatus::Completed, WorkflowStatus::Failed] {
            let storage = Arc::new(MemoryStorage::new());
            let service = WorkflowService::new(storage.clone());
            storage
                .save_workflow_instance(
                    &namespace,
                    0,
                    vec![],
                    WorkflowInstance {
                        id: format!("{status:?}-workflow"),
                        workflow_def_id: "scheduled-workflow".to_string(),
                        version: 0,
                        status,
                        trigger_input: None,
                        pinned_worker_host: None,
                        tasks: HashMap::new(),
                        verifier_states: HashMap::new(),
                    },
                )
                .await
                .unwrap();

            assert!(
                !service
                    .has_active_workflow_instance_for_def(&namespace, "scheduled-workflow")
                    .await
                    .unwrap()
            );
        }
    }

    #[tokio::test]
    async fn list_workflows_filters_by_status() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        let completed = WorkflowInstance {
            id: "completed-workflow".to_string(),
            workflow_def_id: "workflow-1".to_string(),
            version: 0,
            status: WorkflowStatus::Completed,
            trigger_input: None,
            pinned_worker_host: None,
            tasks: HashMap::new(),
            verifier_states: HashMap::new(),
        };
        let mut running = completed.clone();
        running.id = "running-workflow".to_string();
        running.status = WorkflowStatus::Running;

        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                completed,
            )
            .await
            .unwrap();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                running,
            )
            .await
            .unwrap();

        let page = service
            .list_workflows(
                &crate::core::namespace::test_namespace(),
                Some(WorkflowStatus::Running),
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(page.workflows.len(), 1);
        assert_eq!(page.workflows[0].id, "running-workflow");
        assert_eq!(page.workflows[0].workflow_def_id, "workflow-1");
        assert_eq!(page.workflows[0].status, WorkflowStatus::Running);
        assert_eq!(page.workflows[0].total_task_count, 0);
        assert_eq!(page.workflows[0].completed_task_count, 0);
        assert_eq!(page.next_cursor, None);
    }

    #[tokio::test]
    async fn list_workflow_defs_includes_invoked_and_never_invoked_definitions() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        let mut invoked = workflow_def("invoked");
        invoked.description = "Has been run".to_string();
        let mut never_invoked = workflow_def("neverinvoked");
        never_invoked.description = "Ready to run".to_string();
        service
            .create_workflow_def(&crate::core::namespace::test_namespace(), invoked)
            .await
            .unwrap();
        service
            .create_workflow_def(&crate::core::namespace::test_namespace(), never_invoked)
            .await
            .unwrap();

        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                WorkflowInstance {
                    id: "invoked-1".to_string(),
                    workflow_def_id: "invoked".to_string(),
                    version: 0,
                    status: WorkflowStatus::Completed,
                    trigger_input: None,
                    pinned_worker_host: None,
                    tasks: HashMap::new(),
                    verifier_states: HashMap::new(),
                },
            )
            .await
            .unwrap();

        let result = service
            .list_workflow_defs(&crate::core::namespace::test_namespace())
            .await
            .unwrap();
        assert_eq!(result.len(), 2);

        let invoked = result
            .iter()
            .find(|summary| summary.id == "invoked")
            .unwrap();
        assert_eq!(invoked.description, "Has been run");
        assert!(invoked.created_at_epoch_ms > 0);
        assert!(invoked.last_invoked_at_epoch_ms.is_some());

        let never_invoked = result
            .iter()
            .find(|summary| summary.id == "neverinvoked")
            .unwrap();
        assert_eq!(never_invoked.description, "Ready to run");
        assert!(never_invoked.created_at_epoch_ms > 0);
        assert_eq!(never_invoked.last_invoked_at_epoch_ms, None);
    }

    #[tokio::test]
    async fn list_workflow_events_returns_none_for_unknown_instance() {
        let service = WorkflowService::new(Arc::new(MemoryStorage::new()));

        let events = service
            .list_workflow_events(
                &crate::core::namespace::test_namespace(),
                "missing",
                None,
                None,
            )
            .await
            .unwrap();

        assert!(events.is_none());
    }

    #[tokio::test]
    async fn input_needed_task_result_is_not_reported_as_running() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        service
            .create_workflow_def(
                &crate::core::namespace::test_namespace(),
                WorkflowDef {
                    id: "workflow1".to_string(),
                    description: String::new(),
                    tasks: vec![agent_task_def("taska")],
                    data_bindings: vec![],
                },
            )
            .await
            .unwrap();
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                WorkflowInstance {
                    id: "input-workflow".to_string(),
                    workflow_def_id: "workflow1".to_string(),
                    version: 0,
                    status: WorkflowStatus::InputNeeded,
                    trigger_input: None,
                    pinned_worker_host: Some(WorkerHostId::new("test-host")),
                    tasks: HashMap::from([(
                        "taska[1]".to_string(),
                        TaskInstance {
                            task_def_id: "taska".to_string(),
                            status: TaskStatus::InputNeeded {
                                input_request: "Which release channel?".to_string(),
                            },
                            satisfaction_status: TaskSatisfactionStatus::Pending,
                            human_input: None,
                            input_data: vec![],
                            input_mapping: vec![],
                            output_data: None,
                            generation_index: 1,
                            verifier_metadata: None,
                        },
                    )]),
                    verifier_states: HashMap::new(),
                },
            )
            .await
            .unwrap();

        let result = service
            .get_task_result(
                &crate::core::namespace::test_namespace(),
                "input-workflow",
                "taska",
            )
            .await
            .unwrap();
        match result {
            TaskResult::InputNeeded {
                input,
                input_request,
                metadata: Some(metadata),
            } => {
                assert_eq!(input, Vec::<serde_json::Value>::new());
                assert_eq!(input_request, "Which release channel?");
                assert_eq!(metadata.task_attempt_id, "taska[1]");
                assert_eq!(metadata.generation_index, 1);
            }
            result => panic!("expected input-needed result, got {result:?}"),
        }

        let tasks = service
            .list_task_results(&crate::core::namespace::test_namespace(), "input-workflow")
            .await
            .unwrap();
        let serialized = serde_json::to_value(&tasks[0]).unwrap();
        assert_eq!(serialized["result"]["status"], "input_needed");
        assert_eq!(
            serialized["result"]["input_request"],
            "Which release channel?"
        );
        assert!(serialized["result"]["description"].is_null());
    }

    #[tokio::test]
    async fn submit_human_input_records_durable_event() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        service
            .create_workflow_def(
                &crate::core::namespace::test_namespace(),
                WorkflowDef {
                    id: "workflow1".to_string(),
                    description: String::new(),
                    tasks: vec![agent_task_def("taska")],
                    data_bindings: vec![],
                },
            )
            .await
            .unwrap();
        let instance = WorkflowInstance {
            id: "input-workflow".to_string(),
            workflow_def_id: "workflow1".to_string(),
            version: 0,
            status: WorkflowStatus::InputNeeded,
            trigger_input: None,
            pinned_worker_host: Some(WorkerHostId::new("test-host")),
            tasks: HashMap::from([(
                "taska[1]".to_string(),
                TaskInstance {
                    task_def_id: "taska".to_string(),
                    status: TaskStatus::InputNeeded {
                        input_request: "need input".to_string(),
                    },
                    satisfaction_status: Default::default(),
                    human_input: None,
                    input_data: vec![],
                    input_mapping: vec![],
                    output_data: None,
                    generation_index: 1,
                    verifier_metadata: None,
                },
            )]),
            verifier_states: HashMap::new(),
        };
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance,
            )
            .await
            .unwrap();

        let task_attempt_id = service
            .submit_human_input(
                &crate::core::namespace::test_namespace(),
                "input-workflow",
                "taska",
                json!({"answer": "continue"}),
            )
            .await
            .unwrap();

        assert_eq!(task_attempt_id, "taska[2]");
        let saved = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "input-workflow")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, WorkflowStatus::Pending);
        assert_eq!(
            saved.pinned_worker_host,
            Some(WorkerHostId::new("test-host"))
        );
        assert!(matches!(
            saved.tasks["taska[1]"].status,
            TaskStatus::InputNeeded { .. }
        ));
        assert_eq!(saved.tasks["taska[2]"].task_def_id, "taska");
        assert_eq!(saved.tasks["taska[2]"].status, TaskStatus::Pending);
        assert_eq!(saved.tasks["taska[2]"].generation_index, 2);
        assert_eq!(
            saved.tasks["taska[2]"].human_input,
            Some(json!({"answer": "continue"}))
        );

        let events = storage
            .list_workflow_instance_events(
                &crate::core::namespace::test_namespace(),
                "input-workflow",
                WorkflowEventPageRequest {
                    limit: 100,
                    cursor: None,
                },
            )
            .await
            .unwrap()
            .items;
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].event,
            WorkflowInstanceEvent::HumanInputSubmitted {
                task_attempt_id,
                continuation_task_attempt_id,
                submitted_input
            } if task_attempt_id == "taska[1]"
                && continuation_task_attempt_id == "taska[2]"
                && submitted_input == &json!({"answer": "continue"})
        ));
    }

    #[tokio::test]
    async fn submit_human_input_rejects_non_waiting_workflow() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                WorkflowInstance {
                    id: "running-workflow".to_string(),
                    workflow_def_id: "workflow1".to_string(),
                    version: 0,
                    status: WorkflowStatus::Running,
                    trigger_input: None,
                    pinned_worker_host: None,
                    tasks: HashMap::new(),
                    verifier_states: HashMap::new(),
                },
            )
            .await
            .unwrap();

        let error = service
            .submit_human_input(
                &crate::core::namespace::test_namespace(),
                "running-workflow",
                "taska",
                json!({"answer": "continue"}),
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("not waiting for input"));
    }

    #[tokio::test]
    async fn retry_task_records_durable_event_and_preserves_pin() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        let instance = WorkflowInstance {
            id: "failed-workflow".to_string(),
            workflow_def_id: "workflow1".to_string(),
            version: 0,
            status: WorkflowStatus::Failed,
            trigger_input: None,
            pinned_worker_host: Some(WorkerHostId::new("test-host")),
            tasks: HashMap::from([
                (
                    "taska[1]".to_string(),
                    TaskInstance {
                        task_def_id: "taska".to_string(),
                        status: TaskStatus::Failed,
                        satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                        human_input: None,
                        input_data: vec![json!({"input": true})],
                        input_mapping: vec![],
                        output_data: Some(json!({"stale": true})),
                        generation_index: 1,
                        verifier_metadata: None,
                    },
                ),
                (
                    "taskb[1]".to_string(),
                    TaskInstance {
                        task_def_id: "taskb".to_string(),
                        status: TaskStatus::Completed,
                        satisfaction_status: TaskSatisfactionStatus::Satisfied,
                        human_input: None,
                        input_data: vec![],
                        input_mapping: vec![],
                        output_data: Some(json!({"ok": true})),
                        generation_index: 1,
                        verifier_metadata: None,
                    },
                ),
            ]),
            verifier_states: HashMap::new(),
        };
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                instance,
            )
            .await
            .unwrap();

        let task_attempt_id = service
            .retry_task(
                &crate::core::namespace::test_namespace(),
                "failed-workflow",
                "taska",
            )
            .await
            .unwrap();

        assert_eq!(task_attempt_id, "taska[1]");
        let saved = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "failed-workflow")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, WorkflowStatus::Pending);
        assert_eq!(
            saved.pinned_worker_host,
            Some(WorkerHostId::new("test-host"))
        );
        assert_eq!(saved.tasks["taska[1]"].status, TaskStatus::Pending);
        assert_eq!(
            saved.tasks["taska[1]"].satisfaction_status,
            TaskSatisfactionStatus::Pending
        );
        assert_eq!(saved.tasks["taska[1]"].output_data, None);
        assert_eq!(saved.tasks["taskb[1]"].status, TaskStatus::Completed);

        let events = storage
            .list_workflow_instance_events(
                &crate::core::namespace::test_namespace(),
                "failed-workflow",
                WorkflowEventPageRequest {
                    limit: 100,
                    cursor: None,
                },
            )
            .await
            .unwrap()
            .items;
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].event,
            WorkflowInstanceEvent::TaskRetryRequested { task_attempt_id }
                if task_attempt_id == "taska[1]"
        ));
    }

    #[tokio::test]
    async fn retry_task_rejects_non_failed_workflow() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                WorkflowInstance {
                    id: "pending-workflow".to_string(),
                    workflow_def_id: "workflow1".to_string(),
                    version: 0,
                    status: WorkflowStatus::Pending,
                    trigger_input: None,
                    pinned_worker_host: Some(WorkerHostId::new("test-host")),
                    tasks: HashMap::new(),
                    verifier_states: HashMap::new(),
                },
            )
            .await
            .unwrap();

        let error = service
            .retry_task(
                &crate::core::namespace::test_namespace(),
                "pending-workflow",
                "taska",
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("is not failed"));
    }

    #[tokio::test]
    async fn force_retry_task_records_reassignment_and_context_loss() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                WorkflowInstance {
                    id: "failed-workflow".to_string(),
                    workflow_def_id: "workflow1".to_string(),
                    version: 0,
                    status: WorkflowStatus::Failed,
                    trigger_input: None,
                    pinned_worker_host: Some(WorkerHostId::new("host-a")),
                    tasks: HashMap::from([(
                        "taska[1]".to_string(),
                        TaskInstance {
                            task_def_id: "taska".to_string(),
                            status: TaskStatus::Failed,
                            satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                            human_input: None,
                            input_data: vec![json!({"input": true})],
                            input_mapping: vec![],
                            output_data: Some(json!({"stale": true})),
                            generation_index: 1,
                            verifier_metadata: None,
                        },
                    )]),
                    verifier_states: HashMap::new(),
                },
            )
            .await
            .unwrap();

        let task_attempt_id = service
            .force_retry_task(
                &crate::core::namespace::test_namespace(),
                "failed-workflow",
                "taska",
                WorkerHostId::new("host-b"),
            )
            .await
            .unwrap();

        assert_eq!(task_attempt_id, "taska[1]");
        let saved = storage
            .get_workflow_instance(&crate::core::namespace::test_namespace(), "failed-workflow")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, WorkflowStatus::Pending);
        assert_eq!(saved.pinned_worker_host, Some(WorkerHostId::new("host-b")));
        assert_eq!(saved.tasks["taska[1]"].status, TaskStatus::Pending);
        assert_eq!(saved.tasks["taska[1]"].output_data, None);

        let events = storage
            .list_workflow_instance_events(
                &crate::core::namespace::test_namespace(),
                "failed-workflow",
                WorkflowEventPageRequest {
                    limit: 100,
                    cursor: None,
                },
            )
            .await
            .unwrap()
            .items;
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].event,
            WorkflowInstanceEvent::TaskForceRetryRequested {
                task_attempt_id,
                previous_host_id,
                target_host_id,
                local_context_may_be_lost,
            } if task_attempt_id == "taska[1]"
                && previous_host_id == &Some(WorkerHostId::new("host-a"))
                && target_host_id == &WorkerHostId::new("host-b")
                && *local_context_may_be_lost
        ));
    }

    #[tokio::test]
    async fn force_retry_task_records_same_host_without_context_loss() {
        let storage = Arc::new(MemoryStorage::new());
        let service = WorkflowService::new(storage.clone());
        storage
            .save_workflow_instance(
                &crate::core::namespace::test_namespace(),
                0,
                vec![],
                WorkflowInstance {
                    id: "failed-workflow".to_string(),
                    workflow_def_id: "workflow1".to_string(),
                    version: 0,
                    status: WorkflowStatus::Failed,
                    trigger_input: None,
                    pinned_worker_host: Some(WorkerHostId::new("host-a")),
                    tasks: HashMap::from([(
                        "taska[1]".to_string(),
                        TaskInstance {
                            task_def_id: "taska".to_string(),
                            status: TaskStatus::Failed,
                            satisfaction_status: TaskSatisfactionStatus::Unsatisfied,
                            human_input: None,
                            input_data: vec![],
                            input_mapping: vec![],
                            output_data: None,
                            generation_index: 1,
                            verifier_metadata: None,
                        },
                    )]),
                    verifier_states: HashMap::new(),
                },
            )
            .await
            .unwrap();

        service
            .force_retry_task(
                &crate::core::namespace::test_namespace(),
                "failed-workflow",
                "taska",
                WorkerHostId::new("host-a"),
            )
            .await
            .unwrap();

        let events = storage
            .list_workflow_instance_events(
                &crate::core::namespace::test_namespace(),
                "failed-workflow",
                WorkflowEventPageRequest {
                    limit: 100,
                    cursor: None,
                },
            )
            .await
            .unwrap()
            .items;
        assert!(matches!(
            &events[0].event,
            WorkflowInstanceEvent::TaskForceRetryRequested {
                previous_host_id,
                target_host_id,
                local_context_may_be_lost,
                ..
            } if previous_host_id == &Some(WorkerHostId::new("host-a"))
                && target_host_id == &WorkerHostId::new("host-a")
                && !*local_context_may_be_lost
        ));
    }
}
