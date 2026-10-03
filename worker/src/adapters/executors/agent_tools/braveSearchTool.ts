import { Type } from '@earendil-works/pi-ai';
import { logger } from '../../../utils/logger.js';
import type { TaskFetch } from '../../../core/ports/TaskSandbox.js';

export function createBraveSearchTool(braveApiKey: string, request: TaskFetch = fetch) {
    return {
        name: "web_search",
        description: "Search the web using Brave Search API",
        label: "Brave Search",
        parameters: Type.Object({
            query: Type.String({ description: "The search query" })
        }),
        execute: async (toolCallId: string, args: any, signal?: AbortSignal) => {
            logger.info(`[BraveSearchTool] Executing web_search for query: ${args.query}`);
            const response = await request(`https://api.search.brave.com/res/v1/web/search?q=${encodeURIComponent(args.query)}`, {
                headers: {
                    "Accept": "application/json",
                    "Accept-Encoding": "gzip",
                    "X-Subscription-Token": braveApiKey
                },
                signal: signal || null
            });

            if (!response.ok) {
                throw new Error(`Brave search failed with status ${response.status}: ${response.statusText}`);
            }

            const data = await response.json() as any;
            const results = (data.web?.results || []).map((r: any) => ({
                title: r.title,
                url: r.url,
                description: r.description
            }));

            return {
                content: [{ type: "text", text: JSON.stringify(results, null, 2) }],
                details: results
            };
        }
    } as any;
}
