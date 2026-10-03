import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { createGondolinSandbox } from '../dist/adapters/GondolinSandbox.js';
import { FunctionExecutor } from '../dist/adapters/executors/FunctionExecutor.js';
import { ApiCallExecutor } from '../dist/adapters/executors/ApiCallExecutor.js';
import { createSandboxCodingTools } from '../dist/adapters/executors/agent_tools/sandboxCodingTools.js';

test('real Gondolin VM enforces task I/O and separates credential environments', {
    skip: process.env.RELAYFOLD_TEST_GONDOLIN !== '1', timeout: 240_000,
}, async () => {
    const workspace = await mkdtemp(path.join(tmpdir(), 'relayfold-sandbox-integration-'));
    const credentials = { getCredential: async name => ({ first: 'first-secret', second: 'second-secret' })[name] };
    const payload = {
        namespace: 'test', workflow_inst_id: 'sandbox-integration', workspace_path: workspace, inputs: [],
        execution_metadata: { sandbox: { network: { allowed_hosts: ['example.com', 'registry.npmjs.org'] } } },
        task: { id: 'first', required_credentials: ['first'], kind: { function: {
            dependencies: [{ name: 'is-number', version: '7.0.0' }],
            code: `import isNumber from 'is-number';
import { writeFileSync, existsSync } from 'node:fs';
export default function run(context) {
    writeFileSync(context.workspacePath + '/result.txt', 'shared output');
    return { credential: process.env.FIRST, contextCredential: context.credentials.first,
        hostVisible: existsSync('/app/package.json'), dependency: isNumber(42), workspace: context.workspacePath };
}`,
        } } },
    };
    let first; let second;
    try {
        first = await createGondolinSandbox(payload, credentials);
        const guestPayload = { ...payload, workspace_path: first.workspacePath };
        const output = await new FunctionExecutor().execute(guestPayload, credentials, {}, first);
        assert.equal(output.status, 'ok', JSON.stringify(output));
        assert.deepEqual(output.output, { credential: 'first-secret', contextCredential: 'first-secret', hostVisible: false, dependency: true, workspace: '/workspace' });
        assert.equal(await readFile(path.join(workspace, 'result.txt'), 'utf8'), 'shared output');
        const tools = createSandboxCodingTools(first);
        const read = await tools.find(t => t.name === 'read').execute('read', { path: 'result.txt' });
        assert.equal(read.content[0].text, 'shared output');
        await tools.find(t => t.name === 'write').execute('write', { path: 'agent.txt', content: 'before edit' });
        await tools.find(t => t.name === 'edit').execute('edit', { path: 'agent.txt', edits: [{ oldText: 'before', newText: 'after' }] });
        assert.equal(await readFile(path.join(workspace, 'agent.txt'), 'utf8'), 'after edit');
        const bash = await tools.find(t => t.name === 'bash').execute('bash', { command: 'printf "%s" "$FIRST"' });
        assert.equal(bash.content[0].text, 'first-secret');
        const apiPayload = { ...guestPayload, task: { ...payload.task, kind: { apiCall: { url: 'https://example.com', method: 'GET' } } } };
        const allowed = await new ApiCallExecutor().execute(apiPayload, credentials, {}, first);
        assert.equal(allowed.status, 'ok', JSON.stringify(allowed));
        apiPayload.task.kind.apiCall.url = 'https://example.org';
        const blocked = await new ApiCallExecutor().execute(apiPayload, credentials, {}, first);
        assert.equal(blocked.status, 'error');
        await first.close(); first = undefined;
        payload.task.required_credentials = ['second'];
        second = await createGondolinSandbox(payload, credentials);
        const env = await second.exec(['/bin/sh', '-c', 'printf "%s:%s" "${FIRST-unset}" "$SECOND"']);
        assert.equal(env.stdout, 'unset:second-secret');
        assert.equal((await second.readFile('/workspace/result.txt')).toString(), 'shared output');
    } finally {
        await first?.close(); await second?.close();
        await rm(workspace, { recursive: true, force: true });
    }
});
