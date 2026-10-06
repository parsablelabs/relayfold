import assert from 'node:assert/strict';
import test from 'node:test';
import { AuthStorage } from '@earendil-works/pi-coding-agent';
import {
    resolveCredentialEnvironment,
    withTaskEnvironment,
} from '../dist/core/TaskEnvironment.js';

test('provider credentials are available for the complete wrapped Agent execution scope', async () => {
    const previousGhToken = process.env.GH_TOKEN;
    const previousOpenAiKey = process.env.OPENAI_API_KEY;
    const env = await resolveCredentialEnvironment(
        payload(['gh_token', 'openai_api_key']),
        credentials({
            gh_token: 'github-secret',
            openai_api_key: 'openai-secret',
        })
    );

    assert.deepEqual(env, {
        GH_TOKEN: 'github-secret',
        OPENAI_API_KEY: 'openai-secret',
    });

    const authStorage = AuthStorage.inMemory();
    await withTaskEnvironment(env, async () => {
        assert.equal(process.env.GH_TOKEN, 'github-secret');
        assert.equal(
            await authStorage.getApiKey('openai'),
            'openai-secret'
        );
    });

    assert.ok(process.env.GH_TOKEN === previousGhToken, 'GH_TOKEN should be restored');
    assert.ok(process.env.OPENAI_API_KEY === previousOpenAiKey, 'OPENAI_API_KEY should be restored');
});

test('Pi resolves provider-standard Gemini and Anthropic environment variables', async () => {
    const authStorage = AuthStorage.inMemory();
    await withTaskEnvironment(
        {
            GEMINI_API_KEY: 'gemini-secret',
            ANTHROPIC_API_KEY: 'anthropic-secret',
        },
        async () => {
            assert.equal(
                await authStorage.getApiKey('google'),
                'gemini-secret'
            );
            assert.equal(
                await authStorage.getApiKey('anthropic'),
                'anthropic-secret'
            );
        }
    );
});

test('first required credential has no special model-authentication behavior', async () => {
    const env = await resolveCredentialEnvironment(
        payload(['gh_token', 'openai_api_key']),
        credentials({
            gh_token: 'first-but-not-a-model-key',
            openai_api_key: 'provider-key',
        })
    );

    await withTaskEnvironment(env, async () => {
        const authStorage = AuthStorage.inMemory();
        assert.equal(
            await authStorage.getApiKey('openai'),
            'provider-key'
        );
        assert.equal(authStorage.getAuthStatus('gh').configured, false);
    });
});

test('missing explicitly required credential fails before execution scope starts', async () => {
    await assert.rejects(
        resolveCredentialEnvironment(
            payload(['missing_key']),
            credentials({})
        ),
        /Missing required credential: missing_key/
    );
});

test('Pi resolves no model credential when no normal authentication source is configured', async () => {
    const previous = process.env.OPENAI_API_KEY;
    delete process.env.OPENAI_API_KEY;
    try {
        const authStorage = AuthStorage.inMemory();
        assert.equal(await authStorage.getApiKey('openai'), undefined);
        assert.deepEqual(authStorage.getAuthStatus('openai'), {
            configured: false,
        });
    } finally {
        if (previous === undefined) {
            delete process.env.OPENAI_API_KEY;
        } else {
            process.env.OPENAI_API_KEY = previous;
        }
    }
});

function payload(requiredCredentials) {
    return {
        task: {
            required_credentials: requiredCredentials,
        },
    };
}

function credentials(values) {
    return {
        async getCredential(name) {
            return values[name];
        },
    };
}
