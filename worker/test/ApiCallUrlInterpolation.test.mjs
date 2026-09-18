import assert from 'node:assert/strict';
import test from 'node:test';
import { ApiCallRequestResolver } from '../dist/core/ApiCallRequestResolver.js';

const credentials = {
    async getCredential() {
        return undefined;
    },
};

const resolver = new ApiCallRequestResolver(credentials);

async function resolveUrl(url, inputs) {
    const request = await resolver.resolveRequest({ url }, inputs, []);
    return request.url;
}

test('resolves nested input properties and array indexes as URL components', async () => {
    const result = await resolveUrl(
        'https://api.example.com/${inputs[0].portfolio.symbols[1]}?active=${inputs[0].active}&limit=${inputs[1]}',
        [{ portfolio: { symbols: ['AAPL', 'BRK/B'] }, active: true }, 25],
    );

    assert.equal(
        result,
        'https://api.example.com/BRK%2FB?active=true&limit=25',
    );
});

test('resolves repeated expressions in a single pass', async () => {
    const result = await resolveUrl(
        'https://api.example.com/${inputs[0].ticker}/${inputs[0].ticker}',
        [{ ticker: '${inputs[1]}' }, 'AAPL'],
    );

    assert.equal(
        result,
        'https://api.example.com/%24%7Binputs%5B1%5D%7D/%24%7Binputs%5B1%5D%7D',
    );
});

test('supports escaped expressions and ordinary dollar signs', async () => {
    const result = await resolveUrl(
        'https://api.example.com/$${inputs[0].ticker}?price=$100',
        [{ ticker: 'AAPL' }],
    );

    assert.equal(result, 'https://api.example.com/${inputs[0].ticker}?price=$100');
});

test('rejects missing input values', async () => {
    await assert.rejects(
        resolveUrl('https://api.example.com/${inputs[1].ticker}', [{ ticker: 'AAPL' }]),
        /references a missing value/,
    );

    await assert.rejects(
        resolveUrl('https://api.example.com/${inputs[0].missing}', [{ ticker: 'AAPL' }]),
        /references a missing value/,
    );

    await assert.rejects(
        resolveUrl('https://api.example.com/${inputs[0].symbols.length}', [{ symbols: ['AAPL'] }]),
        /references a missing value/,
    );
});

test('rejects non-scalar input values', async () => {
    for (const value of [null, { ticker: 'AAPL' }, ['AAPL']]) {
        await assert.rejects(
            resolveUrl('https://api.example.com/${inputs[0].value}', [{ value }]),
            /must resolve to a string, number, or boolean/,
        );
    }
});

test('rejects malformed input expressions and unsupported namespaces', async () => {
    for (const expression of [
        'inputs.foo',
        'inputs[-1]',
        'inputs[01]',
        'inputs[0]["ticker"]',
    ]) {
        await assert.rejects(
            resolveUrl(`https://api.example.com/\${${expression}}`, [{ ticker: 'AAPL' }]),
            /Malformed input expression/,
        );
    }

    await assert.rejects(
        resolveUrl('https://api.example.com/${input.ticker}', [{ ticker: 'AAPL' }]),
        /Only 'inputs' is supported in API call URLs/,
    );

    await assert.rejects(
        resolveUrl('https://api.example.com/${credentials.api_token}', []),
        /Only 'inputs' is supported in API call URLs/,
    );

    await assert.rejects(
        resolver.resolveRequest(
            {
                url: 'https://api.example.com/items',
                headers: { 'X-Ticker': '${inputs[0].ticker}' },
            },
            [{ ticker: 'AAPL' }],
            [],
        ),
        /Only 'credentials' is supported in header values/,
    );
});

test('rejects unterminated URL expressions', async () => {
    await assert.rejects(
        resolveUrl('https://api.example.com/${inputs[0].ticker', [{ ticker: 'AAPL' }]),
        /Malformed interpolation expression in API call URL/,
    );
});
