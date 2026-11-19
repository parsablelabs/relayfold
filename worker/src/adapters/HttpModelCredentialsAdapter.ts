import type { ModelCredentialsPort } from '../core/ports/ModelCredentialsPort.js';

export class HttpModelCredentialsAdapter implements ModelCredentialsPort {
    constructor(
        private readonly baseUrl: string,
        private readonly request: typeof fetch = fetch,
    ) {}

    async getAccessToken(provider: string): Promise<string> {
        let response: Response;
        try {
            response = await this.request(
                `${this.baseUrl.replace(/\/$/, '')}/workers/auth/${encodeURIComponent(provider)}`,
                { method: 'POST', signal: AbortSignal.timeout(65_000) },
            );
        } catch {
            throw new Error('Unable to retrieve OAuth credentials from the orchestrator');
        }
        if (!response.ok) {
            // Do not echo response bodies: a misconfigured endpoint may expose secrets.
            throw new Error(`OAuth credential retrieval failed (HTTP ${response.status}); check orchestrator login and OAuth storage`);
        }
        let credential: unknown;
        try { credential = await response.json(); }
        catch { throw new Error('Invalid OAuth credential response'); }
        if (typeof credential !== 'object' || credential === null
            || !('access_token' in credential) || typeof credential.access_token !== 'string' || !credential.access_token
            || !('expires_at' in credential) || typeof credential.expires_at !== 'number'
            || !Number.isFinite(credential.expires_at) || credential.expires_at <= Date.now()) {
            throw new Error('Invalid or expired OAuth credential response');
        }
        return credential.access_token;
    }
}
