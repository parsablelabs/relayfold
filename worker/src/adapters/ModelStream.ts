import type { StreamFn } from '@earendil-works/pi-agent-core';
import { createAssistantMessageEventStream, type AssistantMessage } from '@earendil-works/pi-ai';

/** Pi's SDK can reject during credential lookup. Agent loops require error events. */
export function withModelStreamErrors(streamFn: StreamFn): StreamFn {
    return async (model, context, options) => {
        try {
            return await streamFn(model, context, options);
        } catch (error) {
            const reason = options?.signal?.aborted ? 'aborted' : 'error';
            const message: AssistantMessage = {
                role: 'assistant', content: [], api: model.api,
                provider: model.provider, model: model.id,
                usage: {
                    input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0,
                    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
                },
                stopReason: reason,
                errorMessage: error instanceof Error ? error.message : String(error),
                timestamp: Date.now(),
            };
            const stream = createAssistantMessageEventStream();
            stream.push({ type: 'error', reason, error: message });
            return stream;
        }
    };
}
