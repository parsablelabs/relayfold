import { Type } from '@earendil-works/pi-ai';
import { logger } from '../../../utils/logger.js';
import type { TaskFetch } from '../../../core/ports/TaskSandbox.js';

export function createHttpRequestTool(request: TaskFetch = fetch) {
    return {
        name: "http_request",
        description: "Make an arbitrary HTTP request to an external API.",
        label: "HTTP Request",
        parameters: Type.Object({
            url: Type.String({ description: "The full URL to request" }),
            method: Type.Optional(Type.String({ description: "The HTTP method (GET, POST, PUT, DELETE). Defaults to GET." })),
            headers: Type.Optional(Type.Record(Type.String(), Type.String(), { description: "Optional HTTP headers as key-value pairs" })),
            body: Type.Optional(Type.String({ description: "Optional request body string (e.g. JSON string). Should not be provided for GET requests." }))
        }),
        execute: async (toolCallId: string, args: any, signal?: AbortSignal) => {
            const method = (args.method || "GET").toUpperCase();
            logger.info(`[HttpRequestTool] Executing ${method} request to ${args.url}`);

            const options: RequestInit = {
                method,
                headers: args.headers || {},
                signal: signal || null
            };

            if (args.body && method !== "GET" && method !== "HEAD") {
                options.body = args.body;
            }

            try {
                const response = await request(args.url, options);
                
                const responseHeaders: Record<string, string> = {};
                response.headers.forEach((value, key) => {
                    responseHeaders[key] = value;
                });

                let responseBody = await response.text();
                // Limit the response size
                const maxLength = 20000;
                if (responseBody.length > maxLength) {
                    responseBody = responseBody.substring(0, maxLength) + "\n\n...(response truncated)";
                }

                const resultPayload = {
                    status: response.status,
                    statusText: response.statusText,
                    headers: responseHeaders,
                    body: responseBody
                };

                return {
                    content: [{ type: "text", text: JSON.stringify(resultPayload, null, 2) }],
                    details: resultPayload
                };
            } catch (err: any) {
                logger.error(`[HttpRequestTool] Request failed: ${err.message}`);
                throw new Error(`HTTP request failed: ${err.message}`);
            }
        }
    } as any;
}
