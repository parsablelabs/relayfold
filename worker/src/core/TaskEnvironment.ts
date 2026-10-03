import type { TaskExecutionPayload } from './models/TaskDef.js';
import type { CredentialsPort } from './ports/CredentialsPort.js';
import { findEnvKeys } from '@earendil-works/pi-ai';

export async function resolveCredentialEnvironment(
    payload: TaskExecutionPayload,
    credentialsPort: CredentialsPort
): Promise<Record<string, string>> {
    const env: Record<string, string> = {};

    for (const credentialName of payload.task.required_credentials) {
        const value = await credentialsPort.getCredential(credentialName);
        if (value === undefined) {
            throw new Error(`Missing required credential: ${credentialName}`);
        }
        env[credentialName.toUpperCase()] = value;
    }

    return env;
}

export async function withTaskEnvironment<T>(
    env: Record<string, string>,
    run: () => Promise<T>
): Promise<T> {
    const restore = applyTaskEnvironment(env);
    try {
        return await run();
    } finally {
        restore();
    }
}

/** Resolve Pi's provider naming synchronously, without retaining a shared environment. */
export function resolveTaskProviderApiKey(provider: string, env: Record<string, string>): string {
    const restore = applyTaskEnvironment(env);
    try {
        const name = findEnvKeys(provider)?.find(name => Boolean(env[name]));
        if (!name) throw new Error(`No API key declared in required_credentials for provider: ${provider}`);
        return env[name]!;
    } finally {
        restore();
    }
}

function applyTaskEnvironment(env: Record<string, string>): () => void {
    const previous = new Map<string, string | undefined>();

    for (const [name, value] of Object.entries(env)) {
        previous.set(name, process.env[name]);
        process.env[name] = value;
    }

    return () => {
        for (const [name, value] of previous.entries()) {
            if (value === undefined) {
                delete process.env[name];
            } else {
                process.env[name] = value;
            }
        }
    };
}
