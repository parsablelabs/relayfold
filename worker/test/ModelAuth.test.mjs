import assert from 'node:assert/strict';
import test from 'node:test';
import { createAgentSession, DefaultResourceLoader, SessionManager, SettingsManager } from '@earendil-works/pi-coding-agent';
import { withModelStreamErrors } from '../dist/adapters/ModelStream.js';
import { createModelRuntime } from '../dist/adapters/ModelRuntimeAdapter.js';
import { HttpModelCredentialsAdapter } from '../dist/adapters/HttpModelCredentialsAdapter.js';

test('explicit reasoning level overrides a reused session while omission preserves it', async () => {
    const modelRuntime = await createModelRuntime({ getAccessToken: async () => { throw new Error('No model request expected'); } });
    const settingsManager = SettingsManager.inMemory({ defaultThinkingLevel: 'medium' });
    const resourceLoader = new DefaultResourceLoader({
        cwd: '/tmp', agentDir: '/tmp/relayfold-thinking-level-test', settingsManager,
        noExtensions: true, noSkills: true, noPromptTemplates: true,
        noThemes: true, noContextFiles: true,
    });
    await resourceLoader.reload();
    const sessionManager = SessionManager.inMemory();
    sessionManager.appendModelChange('openai-codex', 'gpt-6.1-sol');
    sessionManager.appendThinkingLevelChange('low');
    sessionManager.appendMessage({ role: 'user', content: 'Previous task input', timestamp: Date.now() });
    for (const [requested, expected] of [[undefined, 'low'], ['high', 'high']]) {
        const { session } = await createAgentSession({
            model: modelRuntime.getModel('openai-codex', 'gpt-6.1-sol'), modelRuntime,
            sessionManager, settingsManager, resourceLoader, thinkingLevel: requested,
        });
        try { assert.equal(session.thinkingLevel, expected); }
        finally { session.dispose(); }
    }
});

test('credential failure completes Pi retry handling instead of leaving prompt pending', { timeout: 5000 }, async () => {
    let attempts = 0;
    const settingsManager = SettingsManager.inMemory({ retry: { enabled: true, maxRetries: 1, baseDelayMs: 1 } });
    const resourceLoader = new DefaultResourceLoader({
        cwd: '/tmp', agentDir: '/tmp/relayfold-model-auth-test', settingsManager,
        noExtensions: true, noSkills: true, noPromptTemplates: true,
        noThemes: true, noContextFiles: true,
    });
    await resourceLoader.reload();
    const modelRuntime = await createModelRuntime({ getAccessToken: async () => {
        attempts++;
        throw new Error('Codex credential retrieval failed (HTTP 503)');
    } });
    const { session } = await createAgentSession({
        model: modelRuntime.getModel('openai-codex', 'gpt-6.1-sol'),
        modelRuntime,
        sessionManager: SessionManager.inMemory(),
        settingsManager, resourceLoader,
    });
    session.agent.streamFunction = withModelStreamErrors(session.agent.streamFunction);
    try {
        await session.prompt('Hello');
        assert.equal(attempts, 2);
        assert.match(session.agent.state.errorMessage, /credential retrieval failed/);
        assert.equal(session.agent.state.isStreaming, false);
    } finally { session.dispose(); }
});

test('Pi model runtime retrieves Codex access credentials on each request and keeps API-key auth', async () => {
    let count = 0;
    const runtime = await createModelRuntime({ getAccessToken: async provider => {
        assert.equal(provider, 'openai-codex');
        return `token-${++count}`;
    } });
    const model = runtime.getModel('openai-codex', 'gpt-6.1-sol');
    assert.ok(model);
    // AgentSession.prompt performs this synchronous check before any request.
    assert.equal(runtime.hasConfiguredAuth(model.provider), true);
    assert.equal(count, 0);
    assert.equal((await runtime.getAuth(model)).auth.apiKey, 'token-1');
    assert.equal((await runtime.getAuth(model)).auth.apiKey, 'token-2');
    assert.deepEqual(await runtime.listCredentials(), []);
    const previous = process.env.OPENAI_API_KEY;
    process.env.OPENAI_API_KEY = 'existing-api-key';
    try {
        assert.ok(await runtime.checkAuth('openai'));
        assert.equal((await runtime.getAuth('openai')).auth.apiKey, 'existing-api-key');
    }
    finally { if (previous === undefined) delete process.env.OPENAI_API_KEY; else process.env.OPENAI_API_KEY = previous; }
});

test('GPT-6.1 Codex transport uses fresh orchestrator tokens and their account claims', async () => {
    let count = 0;
    const tokens = [1, 2].map(index => `header.${Buffer.from(JSON.stringify({
        'https://api.openai.com/auth': { chatgpt_account_id: `account-${index}` },
    })).toString('base64url')}.signature`);
    const runtime = await createModelRuntime({ getAccessToken: async () => tokens[count++] });
    const model = runtime.getModel('openai-codex', 'gpt-6.1-sol');
    assert.ok(model);
    for (let index = 0; index < tokens.length; index++) {
        const stream = runtime.streamSimple(model, {
            messages: [{ role: 'user', content: 'Hello', timestamp: Date.now() }],
        }, {
            transport: 'sse',
            onPayload: body => { assert.equal(body.model, 'gpt-6.1-sol'); },
            fetch: async (_url, options) => {
                const headers = new Headers(options.headers);
                assert.equal(headers.get('authorization'), `Bearer ${tokens[index]}`);
                assert.equal(headers.get('chatgpt-account-id'), `account-${index + 1}`);
                return new Response(`data: ${JSON.stringify({
                    type: 'response.completed',
                    response: { status: 'completed', output: [], usage: { input_tokens: 1, output_tokens: 0 } },
                })}\n\n`, { headers: { 'content-type': 'text/event-stream' } });
            },
        });
        const result = await stream.result();
        assert.equal(result.stopReason, 'stop', result.errorMessage);
    }
    assert.equal(count, 2);
    assert.deepEqual(await runtime.listCredentials(), []);
});

test('HTTP adapter validates credentials and avoids leaking error bodies', async () => {
    const adapter = new HttpModelCredentialsAdapter('http://orchestrator:3001/', async (url, options) => {
        assert.equal(url, 'http://orchestrator:3001/workers/auth/openai-codex');
        assert.equal(options.method, 'POST');
        return Response.json({ access_token: 'token', expires_at: Date.now() + 60000 });
    });
    assert.equal(await adapter.getAccessToken('openai-codex'), 'token');
    for (const body of [{ access_token: 'secret', expires_at: 0 }, { access_token: 123, expires_at: Date.now() + 60000 }]) {
        const invalid = new HttpModelCredentialsAdapter('http://orchestrator', async () => Response.json(body));
        await assert.rejects(invalid.getAccessToken('openai-codex'), /Invalid or expired/);
    }
    const failed = new HttpModelCredentialsAdapter('http://orchestrator', async () => new Response('secret-refresh-token', { status: 503 }));
    await assert.rejects(failed.getAccessToken('openai-codex'), error => !error.message.includes('secret') && error.message.includes('503'));
    const unavailable = await createModelRuntime({ getAccessToken: async () => { throw new Error('Not connected'); } });
    assert.equal(unavailable.hasConfiguredAuth('openai-codex'), true);
    await assert.rejects(unavailable.getAuth('openai-codex'), /Not connected/);
});

test('HTTP adapter uses the requested provider as an encoded URL segment', async () => {
    const adapter = new HttpModelCredentialsAdapter('http://orchestrator:3001/', async url => {
        assert.equal(url, 'http://orchestrator:3001/workers/auth/other%2Fprovider%3Fkey%3Dvalue');
        return Response.json({ error: 'Unsupported OAuth model provider' }, { status: 404 });
    });
    await assert.rejects(adapter.getAccessToken('other/provider?key=value'), /HTTP 404/);
});
