import type { JsonValue, TaskExecutor, TaskExecutionResult } from '../../core/ports/TaskExecutor.js';
import type { TaskExecutionPayload } from '../../core/models/TaskDef.js';
import type { CredentialsPort } from '../../core/ports/CredentialsPort.js';
import { logger } from '../../utils/logger.js';
import { CredentialResolver } from '../../core/CredentialResolver.js';

export class ApiCallExecutor implements TaskExecutor {
    async execute(payload: TaskExecutionPayload, credentialsPort: CredentialsPort): Promise<TaskExecutionResult> {
        if (!('apiCall' in payload.task.kind)) {
            return { status: 'error', message: 'ApiCallExecutor received a non-ApiCall task' };
        }

        const apiCallDef = payload.task.kind.apiCall;
        
        let resolvedHeaders: Record<string, string>;
        try {
            const resolver = new CredentialResolver(credentialsPort);
            resolvedHeaders = await resolver.resolveHeaders(
                apiCallDef.headers ?? {},
                payload.task.required_credentials
            );
        } catch (error) {
            return {
                status: 'error',
                message: `Failed to resolve credentials in headers: ${describeError(error)}`,
            };
        }

        logger.info(`[ApiCallExecutor] Calling API: ${apiCallDef.method} ${apiCallDef.url}`);

        try {
            const response = await fetch(apiCallDef.url, {
                method: apiCallDef.method,
                headers: resolvedHeaders,
            });

            if (!response.ok) {
                return {
                    status: 'error',
                    message: `API request failed with HTTP ${response.status} ${response.statusText}`.trim(),
                };
            }

            const responseHeaders = Object.fromEntries(response.headers.entries());
            const responseText = await response.text();
            const contentType = response.headers.get('content-type');
            let body: JsonValue = responseText;

            if (isJsonMediaType(contentType)) {
                try {
                    body = JSON.parse(responseText) as JsonValue;
                } catch (error) {
                    return {
                        status: 'error',
                        message: `API response declared JSON but could not be parsed: ${describeError(error)}`,
                    };
                }
            }

            return {
                status: 'ok',
                output: {
                    status: response.status,
                    headers: responseHeaders,
                    body,
                },
            };
        } catch (error) {
            return {
                status: 'error',
                message: `API request ${apiCallDef.method} ${apiCallDef.url} failed: ${describeError(error)}`,
            };
        }
    }
}

function isJsonMediaType(contentType: string | null): boolean {
    const mediaType = contentType?.split(';', 1)[0]?.trim();
    return mediaType !== undefined
        && /^application\/(?:json|[a-z0-9!#$&^_.+-]+\+json)$/i.test(mediaType);
}

function describeError(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}
