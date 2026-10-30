import { Type } from '@earendil-works/pi-ai';
import { logger } from '../../../utils/logger.js';
import type { TaskFetch } from '../../../core/ports/TaskSandbox.js';

export function createFetchUrlTool(request: TaskFetch = fetch) {
    return {
        name: "fetch_url",
        description: "Fetch the markdown content of a specific URL. Use this to read the contents of a website or a page.",
        label: "Fetch URL",
        parameters: Type.Object({
            url: Type.String({ description: "The full URL to fetch (e.g., https://www.example.com)" })
        }),
        execute: async (toolCallId: string, args: any, signal?: AbortSignal) => {
            logger.info(`[FetchUrlTool] Executing fetch_url for url: ${args.url}`);
            
            const jinaUrl = `https://r.jina.ai/${args.url}`;
            const response = await request(jinaUrl, {
                headers: {
                    "Accept": "text/markdown"
                },
                signal: signal || null
            });

            if (!response.ok) {
                throw new Error(`Failed to fetch URL with status ${response.status}: ${response.statusText}`);
            }

            const text = await response.text();
            // Limit the length to avoid blowing up the context window completely
            const maxLength = 20000;
            const truncatedText = text.length > maxLength ? text.substring(0, maxLength) + "\n\n...(content truncated)" : text;

            return {
                content: [{ type: "text", text: truncatedText }],
                details: { url: args.url, contentLength: text.length }
            };
        }
    } as any;
}
