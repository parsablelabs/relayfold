import type { Skill } from '@earendil-works/pi-coding-agent';

export type SandboxExecOptions = {
    cwd?: string;
    stdin?: string;
    signal?: AbortSignal;
    timeoutMs?: number;
    onData?: (data: Buffer) => void;
};

export type SandboxExecResult = {
    exitCode: number;
    stdout: string;
    stderr: string;
};

export type TaskFetch = (url: string, init?: RequestInit) => Promise<Response>;

/** One task attempt's execution and I/O boundary. */
export interface TaskSandbox {
    readonly workspacePath: string;
    readonly signal: AbortSignal;
    readonly skills: Skill[];
    exec(command: string[], options?: SandboxExecOptions): Promise<SandboxExecResult>;
    readFile(path: string): Promise<Buffer>;
    writeFile(path: string, content: string): Promise<void>;
    mkdir(path: string): Promise<void>;
    fetch: TaskFetch;
    close(): Promise<void>;
}
