import path from 'node:path';
import { VM, RealFSProvider, ReadonlyProvider, createHttpHooks, type VMOptions } from '@earendil-works/gondolin';
import type { Skill } from '@earendil-works/pi-coding-agent';
import type { TaskSandbox, SandboxExecOptions, SandboxExecResult } from '../core/ports/TaskSandbox.js';
import type { TaskExecutionPayload } from '../core/models/TaskDef.js';
import type { CredentialsPort } from '../core/ports/CredentialsPort.js';
import { resolveCredentialEnvironment } from '../core/TaskEnvironment.js';
import { PiResourceToolProvider } from './executors/agent_tools/PiResourceToolProvider.js';
import { selectApprovedSkills } from './executors/agent_tools/skillSelection.js';

type VmFactory = (options: VMOptions) => Promise<VM>;

export async function createGondolinSandbox(
    payload: TaskExecutionPayload,
    credentials: CredentialsPort,
    createVm: VmFactory = VM.create,
): Promise<TaskSandbox> {
    const definition = payload.execution_metadata?.sandbox;
    if (!definition) throw new Error('Missing workflow sandbox definition');
    const env = await resolveCredentialEnvironment(payload, credentials);
    const mounts: NonNullable<VMOptions['vfs']>['mounts'] = {
        '/workspace': new RealFSProvider(payload.workspace_path),
    };
    let skills: Skill[] = [];
    if ('agent' in payload.task.kind && payload.task.kind.agent.skills.length > 0) {
        const resources = await new PiResourceToolProvider({ skillsOnly: true }).loadResources();
        const selected = selectApprovedSkills(resources.skills, payload.task.kind.agent.skills);
        if (selected.unavailableApprovedSkillNames.length) {
            throw new Error(`Requested agent skills are not available: ${selected.unavailableApprovedSkillNames.join(', ')}`);
        }
        skills = selected.approvedSkills.map((skill, index) => {
            const guestDir = `/skills/${index}`;
            mounts[guestDir] = new ReadonlyProvider(new RealFSProvider(skill.baseDir));
            return { ...skill, baseDir: guestDir, filePath: path.posix.join(guestDir, path.basename(skill.filePath)) };
        });
    }
    const { httpHooks } = createHttpHooks({ allowedHosts: definition.network?.allowed_hosts ?? [] });
    const vm = await createVm({
        env,
        httpHooks,
        dns: { mode: 'synthetic' },
        allowWebSockets: false,
        startTimeoutMs: (payload.task.timeout_secs ?? 300) * 1000,
        vfs: { mounts },
    });
    try {
        await vm.start();
    } catch (error) {
        await vm.close();
        throw error;
    }
    return new GondolinSandbox(vm, skills);
}

export class GondolinSandbox implements TaskSandbox {
    readonly workspacePath = '/workspace';
    private readonly lifetime = new AbortController();
    readonly signal = this.lifetime.signal;
    private queue: Promise<unknown> = Promise.resolve();
    private closed = false;
    private closing: Promise<void> | undefined;

    constructor(private readonly vm: VM, readonly skills: Skill[] = []) {}

    exec(command: string[], options: SandboxExecOptions = {}): Promise<SandboxExecResult> {
        // Gondolin's guest executes one command at a time.
        const result = this.queue.then(() => this.execute(command, options));
        this.queue = result.catch(() => undefined);
        return result;
    }

    private async execute(command: string[], options: SandboxExecOptions): Promise<SandboxExecResult> {
        if (this.closed) throw new Error('Task sandbox is closed');
        const controller = new AbortController();
        const abort = () => { controller.abort(); void this.close().catch(() => undefined); };
        options.signal?.addEventListener('abort', abort, { once: true });
        if (options.signal?.aborted) abort();
        const timer = options.timeoutMs ? setTimeout(abort, options.timeoutMs) : undefined;
        try {
            const proc = this.vm.exec(command, {
                cwd: options.cwd ?? this.workspacePath,
                ...(options.stdin !== undefined ? { stdin: options.stdin } : {}),
                signal: controller.signal,
                ...(options.onData ? { stdout: 'pipe' as const, stderr: 'pipe' as const } : {}),
            });
            if (options.onData) {
                for await (const chunk of proc.output()) options.onData(chunk.data);
            }
            const result = await proc;
            return { exitCode: result.exitCode, stdout: result.stdout, stderr: result.stderr };
        } catch (error) {
            if (controller.signal.aborted) throw new Error('Sandbox command aborted or timed out');
            throw error;
        } finally {
            if (timer) clearTimeout(timer);
            options.signal?.removeEventListener('abort', abort);
        }
    }

    async readFile(filePath: string): Promise<Buffer> {
        if (this.closed) throw new Error('Task sandbox is closed');
        return this.vm.fs.readFile(filePath);
    }

    async writeFile(filePath: string, content: string): Promise<void> {
        if (this.closed) throw new Error('Task sandbox is closed');
        await this.vm.fs.writeFile(filePath, content);
    }

    async mkdir(directory: string): Promise<void> {
        if (this.closed) throw new Error('Task sandbox is closed');
        await this.vm.fs.mkdir(directory, { recursive: true });
    }

    fetch = async (url: string, init: RequestInit = {}): Promise<Response> => {
        const request = new Request(url, init);
        const body = request.body ? Buffer.from(await request.arrayBuffer()).toString('base64') : undefined;
        const result = await this.exec(['/usr/bin/node', '--input-type=module', '-e', FETCH_SCRIPT], {
            stdin: JSON.stringify({ url: request.url, method: request.method, headers: [...request.headers], body }),
            ...(init.signal ? { signal: init.signal } : {}),
        });
        if (result.exitCode !== 0) throw new Error('Sandbox HTTP request failed');
        const response = JSON.parse(result.stdout);
        if (response.error) throw new Error(`Sandbox HTTP request failed: ${response.error}`);
        const emptyBody = request.method === 'HEAD' || [204, 205, 304].includes(response.status);
        return new Response(emptyBody ? null : Buffer.from(response.body, 'base64'), {
            status: response.status, statusText: response.statusText, headers: response.headers,
        });
    };

    close(): Promise<void> {
        this.closed = true;
        this.lifetime.abort();
        this.closing ??= this.vm.close();
        return this.closing;
    }
}

const FETCH_SCRIPT = `
import { readFileSync } from 'node:fs';
try {
    const request = JSON.parse(readFileSync(0, 'utf8'));
    const response = await fetch(request.url, {
        method: request.method, headers: request.headers,
        ...(request.body !== undefined ? { body: Buffer.from(request.body, 'base64') } : {}),
    });
    const body = Buffer.from(await response.arrayBuffer()).toString('base64');
    console.log(JSON.stringify({ status: response.status, statusText: response.statusText, headers: [...response.headers], body }));
} catch (error) { console.log(JSON.stringify({ error: error.message })); }
`;
