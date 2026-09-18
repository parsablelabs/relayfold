import assert from 'node:assert/strict';
import http from 'node:http';
import test from 'node:test';
import { ApiCallExecutor } from '../dist/adapters/executors/ApiCallExecutor.js';

const credentials = {
    async getCredential() {
        return undefined;
    },
};

test('forwards literal headers and parses JSON responses', async () => {
    await withServer((request, response) => {
        response.writeHead(200, {
            'content-type': 'application/problem+json; charset=utf-8',
            'x-response-id': 'response-1',
        });
        response.end(JSON.stringify({
            accept: request.headers.accept,
            clientVersion: request.headers['x-client-version'],
        }));
    }, async (url) => {
        const result = await execute(url, {
            Accept: 'application/json',
            'X-Client-Version': '1',
        });

        assert.equal(result.status, 'ok');
        assert.equal(result.output.status, 200);
        assert.equal(result.output.headers['content-type'], 'application/problem+json; charset=utf-8');
        assert.equal(result.output.headers['x-response-id'], 'response-1');
        assert.deepEqual(result.output.body, {
            accept: 'application/json',
            clientVersion: '1',
        });
    });
});

test('interpolates URL-encoded input values into the request URL', async () => {
    await withServer((request, response) => {
        response.writeHead(200, { 'content-type': 'application/json' });
        response.end(JSON.stringify({ requestUrl: request.url }));
    }, async (url) => {
        const result = await execute(
            `${url}/quotes/\${inputs[0].ticker}?market=\${inputs[0].markets[1]}`,
            undefined,
            [{ ticker: 'BRK/B', markets: ['NYSE', 'US stocks'] }],
        );

        assert.equal(result.status, 'ok');
        assert.equal(result.output.body.requestUrl, '/quotes/BRK%2FB?market=US%20stocks');
    });
});

test('does not make a request when URL interpolation fails', async () => {
    let requestCount = 0;

    await withServer((_request, response) => {
        requestCount++;
        response.writeHead(200);
        response.end();
    }, async (url) => {
        const result = await execute(
            `${url}/quotes/\${inputs[1].ticker}`,
            undefined,
            [{ ticker: 'AAPL' }],
        );

        assert.equal(result.status, 'error');
        assert.match(result.message, /references a missing value/);
        assert.equal(requestCount, 0);
    });
});

test('returns non-JSON response bodies as strings when headers are omitted', async () => {
    await withServer((_request, response) => {
        response.writeHead(200, { 'content-type': 'text/plain' });
        response.end('plain response');
    }, async (url) => {
        const result = await execute(url);

        assert.equal(result.status, 'ok');
        assert.equal(result.output.status, 200);
        assert.equal(result.output.headers['content-type'], 'text/plain');
        assert.equal(result.output.body, 'plain response');
    });
});

test('fails non-success HTTP responses', async () => {
    await withServer((_request, response) => {
        response.writeHead(503, 'Service Unavailable');
        response.end('try later');
    }, async (url) => {
        const result = await execute(url);

        assert.deepEqual(result, {
            status: 'error',
            message: 'API request failed with HTTP 503 Service Unavailable',
        });
    });
});

test('fails malformed responses that declare a JSON content type', async () => {
    await withServer((_request, response) => {
        response.writeHead(200, { 'content-type': 'application/json' });
        response.end('{not valid json');
    }, async (url) => {
        const result = await execute(url);

        assert.equal(result.status, 'error');
        assert.match(result.message, /declared JSON but could not be parsed/);
    });
});

test('fails network errors with a human-readable reason', async () => {
    const server = http.createServer();
    await listen(server);
    const address = server.address();
    assert(address && typeof address === 'object');
    const url = `http://127.0.0.1:${address.port}`;
    await close(server);

    const result = await execute(url);

    assert.equal(result.status, 'error');
    assert.match(result.message, new RegExp(`API request GET ${url} failed:`));
});

test('fails invalid request configuration with a human-readable reason', async () => {
    const result = await execute('not a URL');

    assert.equal(result.status, 'error');
    assert.match(result.message, /API request GET not a URL failed:/);
});

async function execute(url, headers, inputs = []) {
    const apiCall = { url, method: 'GET' };
    if (headers !== undefined) {
        apiCall.headers = headers;
    }

    return await new ApiCallExecutor().execute(
        {
            namespace: 'default',
            workflow_inst_id: 'workflow-1',
            task: {
                id: 'fetch-data',
                kind: { apiCall },
                required_credentials: [],
            },
            workspace_path: '/tmp/relayfold/workflow-1/taskid-fetch-data',
            inputs,
        },
        credentials
    );
}

async function withServer(handler, run) {
    const server = http.createServer(handler);
    await listen(server);
    const address = server.address();
    assert(address && typeof address === 'object');

    try {
        await run(`http://127.0.0.1:${address.port}`);
    } finally {
        await close(server);
    }
}

async function listen(server) {
    await new Promise((resolve, reject) => {
        server.once('error', reject);
        server.listen(0, '127.0.0.1', resolve);
    });
}

async function close(server) {
    server.closeAllConnections();
    await new Promise((resolve, reject) => {
        server.close((error) => error ? reject(error) : resolve());
    });
}
