import type { TaskExecutionPayload } from '../models/TaskDef.js';
import type { CredentialsPort } from './CredentialsPort.js';
import type { JsonValue } from 'type-fest';
import type { SessionStore } from './SessionStore.js';
import type { TaskSandbox } from './TaskSandbox.js';
export type { JsonValue };

export type TaskExecutionResult =
    | { status: 'ok'; output: JsonValue }
    | { status: 'error'; message: string; code?: string | null }
    | { status: 'input_needed'; description: string };

export interface TaskExecutor {
    execute(payload: TaskExecutionPayload, credentialsPort: CredentialsPort, sessionStore: SessionStore, sandbox?: TaskSandbox): Promise<TaskExecutionResult>;
}
