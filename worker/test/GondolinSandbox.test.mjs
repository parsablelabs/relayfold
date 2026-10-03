import assert from 'node:assert/strict';
import test from 'node:test';
import { createGondolinSandbox, GondolinSandbox } from '../dist/adapters/GondolinSandbox.js';
import { executeTask } from '../dist/adapters/executeTask.js';
import { FunctionExecutor } from '../dist/adapters/executors/FunctionExecutor.js';
import { ApiCallExecutor } from '../dist/adapters/executors/ApiCallExecutor.js';
import { createSandboxCodingTools } from '../dist/adapters/executors/agent_tools/sandboxCodingTools.js';
import { resolveTaskProviderApiKey } from '../dist/core/TaskEnvironment.js';
import { mkdtemp, mkdir, writeFile, access, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { PiResourceToolProvider } from '../dist/adapters/executors/agent_tools/PiResourceToolProvider.js';

test('sandbox skill discovery never executes host Pi extensions', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'relayfold-sandbox-extension-'));
    const extension = path.join(root, 'extension.ts'); const marker = path.join(root, 'executed');
    try {
        await writeFile(extension, `import { writeFileSync } from 'node:fs';
writeFileSync(${JSON.stringify(marker)}, 'executed');
export default function () {}`);
        const packageDir = path.join(root, 'node_modules', '@acme', 'pi-skill');
        const skillDir = path.join(packageDir, 'skills', 'sample');
        await mkdir(skillDir, { recursive: true });
        await writeFile(path.join(packageDir, 'package.json'), JSON.stringify({
            name: '@acme/pi-skill', pi: { extensions: [extension], skills: ['skills'] },
        }));
        await writeFile(path.join(skillDir, 'SKILL.md'), '---\nname: sample\ndescription: Sample skill\n---\nSample content');
        const resources = await new PiResourceToolProvider({
            cwd: root, agentDir: path.join(root, 'agent'), nodeModulesDir: path.join(root, 'node_modules'),
            extensionPaths: [extension], skillsOnly: true,
        }).loadResources();
        assert.deepEqual(resources.tools, []);
        assert.ok(resources.skills.some(skill => skill.name === 'sample'));
        await assert.rejects(access(marker));
    } finally { await rm(root, { recursive: true, force: true }); }
});

test('sandbox model authentication uses only current task credentials and restores the host environment', () => {
    const previous = process.env.GEMINI_API_KEY;
    process.env.GEMINI_API_KEY = 'other-task-key';
    try {
        assert.throws(() => resolveTaskProviderApiKey('google', {}), /No API key declared/);
        assert.equal(resolveTaskProviderApiKey('google', { GEMINI_API_KEY: 'current-task-key' }), 'current-task-key');
        assert.equal(process.env.GEMINI_API_KEY, 'other-task-key');
    } finally {
        if (previous === undefined) delete process.env.GEMINI_API_KEY;
        else process.env.GEMINI_API_KEY = previous;
    }
});

const credentials = { getCredential: async name => ({ first: 'first-secret', second: 'second-secret' })[name] };
const payload = (required = [], sandbox = {}) => ({
    namespace: 'test', workflow_inst_id: 'run', workspace_path: '/tmp', inputs: [],
    execution_metadata: { sandbox },
    task: { id: 'task', kind: { function: { code: 'export default () => ({})', dependencies: [] } }, required_credentials: required },
});
function fakeSandbox(overrides = {}) {
    return { workspacePath: '/workspace', signal: new AbortController().signal, skills: [], exec: async () => ({ exitCode: 0, stdout: '', stderr: '' }),
        mkdir: async () => {}, writeFile: async () => {}, readFile: async () => Buffer.from('guest data'),
        fetch: async () => new Response('{}'), close: async () => {}, ...overrides };
}

test('fresh task VMs receive only declared credentials and deny egress by default', async () => {
    const options = [];
    const createVm = async configuration => {
        options.push(configuration);
        return { start: async () => {}, close: async () => {} };
    };
    const first = await createGondolinSandbox(payload(['first']), credentials, createVm);
    const second = await createGondolinSandbox(payload(['second']), credentials, createVm);
    assert.deepEqual(options[0].env, { FIRST: 'first-secret' });
    assert.deepEqual(options[1].env, { SECOND: 'second-secret' });
    assert.equal(await options[0].httpHooks.isIpAllowed({ hostname: 'example.com', ip: '93.184.216.34' }), false);
    assert.deepEqual(Object.keys(options[0].vfs.mounts), ['/workspace']);
    assert.equal(options[0].allowWebSockets, false);
    await first.close(); await second.close();
});

test('host policy permits explicit public destinations and blocks private and other hosts', async () => {
    let options;
    await createGondolinSandbox(payload([], { network: { allowed_hosts: ['api.example.com', '*.github.com'] } }), credentials,
        async configuration => { options = configuration; return { start: async () => {}, close: async () => {} }; });
    const allow = options.httpHooks.isIpAllowed;
    assert.equal(await allow({ hostname: 'api.example.com', ip: '93.184.216.34' }), true);
    assert.equal(await allow({ hostname: 'api.github.com', ip: '140.82.112.5' }), true);
    assert.equal(await allow({ hostname: 'api.example.com', ip: '127.0.0.1' }), false);
    assert.equal(await allow({ hostname: 'other.example.com', ip: '93.184.216.34' }), false);
});

test('missing credentials prevent VM creation; startup failure closes the VM', async () => {
    let created = false;
    await assert.rejects(createGondolinSandbox(payload(['missing']), credentials, async () => { created = true; }), /Missing required credential/);
    assert.equal(created, false);
    let closed = false;
    await assert.rejects(createGondolinSandbox(payload(), credentials, async () => ({
        start: async () => { throw new Error('boot failed'); }, close: async () => { closed = true; },
    })), /boot failed/);
    assert.equal(closed, true);
});

test('normal workflows use host execution; sandbox failure never falls back', async () => {
    const ordinary = payload(); delete ordinary.execution_metadata;
    let executed = 0;
    const executor = { execute: async (_p, _c, _s, sandbox) => {
        executed++; assert.equal(sandbox, undefined); return { status: 'ok', output: {} };
    } };
    await executeTask(ordinary, executor, credentials, {}, async () => { throw new Error('must not create'); });
    await assert.rejects(executeTask(payload(), executor, credentials, {}, async () => { throw new Error('no QEMU'); }), /no QEMU/);
    assert.equal(executed, 1);
});

test('VM cleanup runs for success, human input, failure and deadline expiry', async () => {
    for (const status of ['ok', 'input_needed', 'throw', 'timeout']) {
        let closed = 0;
        const p = payload(); p.task.timeout_secs = 0.01;
        const executor = { execute: async dispatched => {
            assert.equal(dispatched.workspace_path, '/workspace');
            if (status === 'throw') throw new Error('task failed');
            if (status === 'timeout') return new Promise(() => {});
            return status === 'ok' ? { status, output: {} } : { status, description: 'question' };
        } };
        const execution = executeTask(p, executor, credentials, {}, async () => fakeSandbox({ close: async () => { closed++; } }));
        if (status === 'throw' || status === 'timeout') await assert.rejects(execution);
        else assert.equal((await execution).status, status);
        assert.ok(closed >= 1);
    }
});

test('command abort tears down the VM rather than merely cancelling the wait', async () => {
    let closed = 0;
    const sandbox = new GondolinSandbox({ close: async () => { closed++; }, exec: (_args, options) =>
        new Promise((_resolve, reject) => options.signal.addEventListener('abort', () => reject(new Error('cancelled')), { once: true })) });
    await assert.rejects(sandbox.exec(['/bin/sleep', '60'], { timeoutMs: 10 }), /timed out/);
    await sandbox.close();
    assert.equal(closed, 1);
    await assert.rejects(sandbox.exec(['/bin/echo', 'hello']), /closed/);
});

test('deadline includes startup and a late VM is closed without executing the task', async () => {
    const p = payload(); p.task.timeout_secs = 0.01;
    let resolveStartup; let executed = false; let closed = false;
    const startup = new Promise(resolve => { resolveStartup = resolve; });
    await assert.rejects(executeTask(p, { execute: async () => { executed = true; } }, credentials, {}, () => startup), /timed out/);
    resolveStartup(fakeSandbox({ close: async () => { closed = true; } }));
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(executed, false); assert.equal(closed, true);
});

test('Function code and package installation execute inside the guest', async () => {
    const written = new Map(); const commands = [];
    const sandbox = fakeSandbox({
        writeFile: async (file, content) => written.set(file, content),
        exec: async (command, options) => {
            commands.push(command);
            if (command[0] === '/usr/bin/node') {
                const context = JSON.parse(options.stdin);
                assert.equal(context.workspacePath, '/workspace');
                assert.deepEqual(context.credentials, { first: 'first-secret' });
                return { exitCode: 0, stdout: '__RELAYFOLD_RESULT__{"status":"ok","output":{"guest":true}}', stderr: '' };
            }
            return { exitCode: 0, stdout: '', stderr: '' };
        },
    });
    const p = payload(['first']); p.task.kind.function.dependencies = [{ name: 'lodash', version: '4.17.21' }];
    const result = await executeTask(p, new FunctionExecutor(), credentials, {}, async () => sandbox);
    assert.deepEqual(result, { status: 'ok', output: { guest: true } });
    assert.ok(written.has('/tmp/relayfold-function/task.mjs'));
    assert.equal(commands[0][0], '/usr/bin/npm');
    assert.ok(commands[0].includes('--ignore-scripts'));
});

test('API Calls use sandbox transport with task credential interpolation', async () => {
    const p = payload(['second']);
    p.task.kind = { apiCall: { url: 'https://api.example.com/data', method: 'GET', headers: { Authorization: 'Bearer ${credentials.second}' } } };
    const sandbox = fakeSandbox({ fetch: async (url, options) => {
        assert.equal(url, 'https://api.example.com/data');
        assert.equal(options.headers.Authorization, 'Bearer second-secret');
        return new Response('{"sandbox":true}', { headers: { 'Content-Type': 'application/json' } });
    } });
    const result = await executeTask(p, new ApiCallExecutor(), credentials, {}, async () => sandbox);
    assert.deepEqual(result.output.body, { sandbox: true });
});

test('Agent read and bash tools delegate to the guest without inheriting host environment', async () => {
    let command;
    const sandbox = fakeSandbox({ exec: async (args, options = {}) => {
        command = args; assert.equal(options.env, undefined);
        options.onData?.(Buffer.from('guest output'));
        return { exitCode: 0, stdout: '', stderr: '' };
    } });
    const tools = createSandboxCodingTools(sandbox);
    const read = await tools.find(t => t.name === 'read').execute('read', { path: 'data.txt' });
    assert.equal(read.content[0].text, 'guest data');
    const result = await tools.find(t => t.name === 'bash').execute('bash', { command: 'echo hello' });
    assert.deepEqual(command, ['/bin/bash', '-lc', 'echo hello']);
    assert.equal(result.content[0].text, 'guest output');
});
