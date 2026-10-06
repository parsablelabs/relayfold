export type TaskKind =
    | { agent: { model_id: string; provider_url: string; prompt: string; tools: string[]; skills: string[];
        ask?: boolean; schema_failure_retry_times?: number, reuse_session?: boolean} }
    | { apiCall: { url: string; method: string; headers?: Record<string, string> } }
    | { function: InlineFunctionTask | ReferencedFunctionTask };

export interface InlineFunctionTask {
    code: string;
    dependencies: FunctionDependency[];
}

export interface ReferencedFunctionTask {
    ref: string;
}

export interface FunctionDependency {
    name: string;
    version: string;
}

export interface TaskDef {
    id: string;
    kind: TaskKind;
    control?: TaskControl;
    timeout_secs?: number;
    input_schemas?: any[];
    output_schema?: any;
    required_credentials: string[];
}

export interface TaskControl {
    verifier?: VerifierControlConfig;
}

export interface VerifierControlConfig {
    max_iterations: number;
    on_exhausted_continue: boolean;
    rerun_from_task_id?: string;
}

export interface LoopFeedbackEntry {
    generation: number;
    feedback: string;
}

export interface LoopExecutionContext {
    generation: number;
    max_iterations: number;
    feedback_history?: LoopFeedbackEntry[];
    previous_output?: any;
}

export interface ExecutionMetadata {
    generation_index?: number;
    loop_context?: LoopExecutionContext;
}

export interface TaskExecutionPayload {
    namespace: string;
    workflow_inst_id: string;
    task: TaskDef;
    workspace_path_suffix?: string;
    workspace_path: string;
    inputs: any[];
    execution_metadata?: ExecutionMetadata;
    input_provided?: string;
}

export type TaskDispatchPayload = Omit<TaskExecutionPayload, 'workspace_path' | 'workspace_path_suffix'> & {
    workspace_path_suffix: string;
};
