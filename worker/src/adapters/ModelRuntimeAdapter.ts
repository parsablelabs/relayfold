import { InMemoryCredentialStore } from '@earendil-works/pi-ai';
import { ModelRuntime } from '@earendil-works/pi-coding-agent';
import type { ModelCredentialsPort } from '../core/ports/ModelCredentialsPort.js';

/** Resolve Codex access tokens through the orchestrator on every model request. */
export async function createModelRuntime(credentials: ModelCredentialsPort): Promise<ModelRuntime> {
    const runtime = await ModelRuntime.create({
        credentials: new InMemoryCredentialStore(),
        modelsPath: null,
        refreshOnCreate: false,
    });
    const codex = runtime.getProvider('openai-codex');
    if (!codex) throw new Error('Pi OpenAI Codex provider is unavailable');
    runtime.registerNativeProvider({
        ...codex,
        auth: {
            apiKey: {
                name: 'Orchestrator-managed Codex subscription',
                // Configuration checks must not fetch or persist access tokens.
                check: async () => ({ type: 'api_key', source: 'orchestrator' }),
                resolve: async () => ({
                    auth: { apiKey: await credentials.getAccessToken('openai-codex') },
                    source: 'orchestrator',
                }),
            },
        },
    });
    await runtime.refresh({ allowNetwork: false });
    return runtime;
}
