import assert from 'node:assert/strict';
import http from 'node:http';
import test from 'node:test';
import { ApiCallExecutor } from '../dist/adapters/executors/ApiCallExecutor.js';

const mockCredentialsPort = {
    async getCredential(name) {
        if (name === 'api_token') return 'secret-token';
        if (name === 'other_token') return 'other-secret';
        if (name === 'nested') return '${credentials.api_token}'; // Test non-recursive
        return undefined;
    },
};

async function executeApiCall(headers, required_credentials = []) {
    const apiCall = { 
        url: 'http://127.0.0.1:0', // Will be replaced in withServer
        method: 'GET',
        headers 
    };

    return await withServer(async (request, response) => {
        response.writeHead(200, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({
            receivedHeaders: request.headers
        }));
    }, async (url) => {
        apiCall.url = url;
        const executor = new ApiCallExecutor();
        return await executor.execute(
            {
                namespace: 'default',
                workflow_inst_id: 'workflow-1',
                task: {
                    id: 'test-task',
                    kind: { apiCall },
                    required_credentials,
                },
                workspace_path: '/tmp/test',
                inputs: [],
            },
            mockCredentialsPort
        );
    });
}

test('one credential embedded in surrounding text', async () => {
    const result = await executeApiCall(
        { Authorization: 'Bearer ${credentials.api_token}' },
        ['api_token']
    );
    assert.equal(result.status, 'ok');
    assert.equal(result.output.body.receivedHeaders.authorization, 'Bearer secret-token');
});

test('multiple and repeated credential references', async () => {
    const result = await executeApiCall(
        { 'X-Multi': '${credentials.api_token} and ${credentials.other_token} and again ${credentials.api_token}' },
        ['api_token', 'other_token']
    );
    assert.equal(result.status, 'ok');
    assert.equal(result.output.body.receivedHeaders['x-multi'], 'secret-token and other-secret and again secret-token');
});

test('$${ escaping and ordinary literal dollar signs', async () => {
    const result = await executeApiCall(
        { 
            'X-Escaped': '$${credentials.api_token}',
            'X-Dollar': 'Amount is $100',
            'X-Double-Dollar': 'Amount is $$100'
        },
        ['api_token']
    );
    assert.equal(result.status, 'ok');
    assert.equal(result.output.body.receivedHeaders['x-escaped'], '${credentials.api_token}');
    assert.equal(result.output.body.receivedHeaders['x-dollar'], 'Amount is $100');
    assert.equal(result.output.body.receivedHeaders['x-double-dollar'], 'Amount is $$100');
});

test('invalid credential names', async () => {
    const result = await executeApiCall(
        { Authorization: 'Bearer ${credentials.123invalid}' },
        ['api_token']
    );
    assert.equal(result.status, 'error');
    assert.match(result.message, /Invalid credential name/);
});

test('malformed and unknown expressions', async () => {
    const result1 = await executeApiCall({ Auth: 'Bearer ${unknown.ns}' });
    assert.equal(result1.status, 'error');
    assert.match(result1.message, /Unknown namespace/);

    const result2 = await executeApiCall({ Auth: 'Bearer ${credentials.name.extra}' });
    assert.equal(result2.status, 'error');
    assert.match(result2.message, /Invalid credential name/);
});

test('undeclared and missing credentials', async () => {
    const result1 = await executeApiCall({ Auth: '${credentials.api_token}' }, []);
    assert.equal(result1.status, 'error');
    assert.match(result1.message, /referenced but not declared/);

    const result2 = await executeApiCall({ Auth: '${credentials.missing_token}' }, ['missing_token']);
    assert.equal(result2.status, 'error');
    assert.match(result2.message, /is unavailable/);
});

test('non-recursive substitution', async () => {
    const result = await executeApiCall(
        { 'X-Nested': '${credentials.nested}' },
        ['nested', 'api_token']
    );
    assert.equal(result.status, 'ok');
    assert.equal(result.output.body.receivedHeaders['x-nested'], '${credentials.api_token}');
});

test('confirmation that secrets are absent from logs and error results', async () => {
    // We already checked that errors include names but not values.
    const result = await executeApiCall({ Auth: '${credentials.missing}' }, ['missing']);
    assert.equal(result.status, 'error');
    assert.ok(!result.message.includes('secret-token'));
    assert.match(result.message, /Credential 'missing' is unavailable/);
});

async function withServer(handler, run) {
    const server = http.createServer(handler);
    await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const url = `http://127.0.0.1:${address.port}`;
    try {
        return await run(url);
    } finally {
        server.close();
    }
}
