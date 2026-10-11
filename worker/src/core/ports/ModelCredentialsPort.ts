/** Request credentials only; renewal and refresh secrets belong to the orchestrator. */
export interface ModelCredentialsPort {
    getAccessToken(provider: string): Promise<string>;
}
