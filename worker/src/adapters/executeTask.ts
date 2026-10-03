import type { TaskExecutionPayload } from '../core/models/TaskDef.js';
import type { CredentialsPort } from '../core/ports/CredentialsPort.js';
import type { SessionStore } from '../core/ports/SessionStore.js';
import type { TaskExecutor, TaskExecutionResult } from '../core/ports/TaskExecutor.js';
import type { TaskSandbox } from '../core/ports/TaskSandbox.js';
import { createGondolinSandbox } from './GondolinSandbox.js';

type SandboxFactory = (payload: TaskExecutionPayload, credentials: CredentialsPort) => Promise<TaskSandbox>;

export async function executeTask(
    payload: TaskExecutionPayload,
    executor: TaskExecutor,
    credentials: CredentialsPort,
    sessions: SessionStore,
    createSandbox: SandboxFactory = createGondolinSandbox,
): Promise<TaskExecutionResult> {
    if (!payload.execution_metadata?.sandbox) return executor.execute(payload, credentials, sessions);
    let sandbox: TaskSandbox | undefined;
    let timedOut = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
        return await Promise.race([
            (async () => {
                sandbox = await createSandbox(payload, credentials);
                if (timedOut) {
                    await sandbox.close();
                    throw new Error('Sandbox task execution timed out');
                }
                return executor.execute({ ...payload, workspace_path: sandbox.workspacePath }, credentials, sessions, sandbox);
            })(),
            new Promise<never>((_, reject) => {
                timer = setTimeout(() => {
                    timedOut = true;
                    reject(new Error('Sandbox task execution timed out'));
                    void sandbox?.close().catch(() => undefined);
                }, (payload.task.timeout_secs ?? 300) * 1000);
            }),
        ]);
    } finally {
        if (timer) clearTimeout(timer);
        await sandbox?.close();
    }
}
