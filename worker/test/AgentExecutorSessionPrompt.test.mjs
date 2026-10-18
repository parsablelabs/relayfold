import assert from 'node:assert/strict';
import test from 'node:test';
import {
    buildAgentPromptParts,
    sessionLoadDiagnostic,
    shouldLoadExistingAgentSession,
    shouldReuseAgentSession,
} from '../dist/adapters/executors/AgentExecutor.js';

test('session load decision is attempt-aware and honors reuse_session false', () => {
    assert.equal(shouldReuseAgentSession(payload({ generation_index: 1 })), true);
    assert.equal(shouldLoadExistingAgentSession(payload({ generation_index: 1 })), false);
    assert.equal(shouldLoadExistingAgentSession(payload({ generation_index: 2 })), true);

    const nonReusable = payload({
        generation_index: 2,
        reuse_session: false,
    });

    assert.equal(shouldReuseAgentSession(nonReusable), false);
    assert.equal(shouldLoadExistingAgentSession(nonReusable), false);
});

test('session load diagnostics include session key and attempt', () => {
    const sessionKey = {
        namespace: '550e8400-e29b-41d4-a716-446655440000',
        workflowInstId: 'workflow-1',
        taskId: 'draft',
    };
    const missing = sessionLoadDiagnostic('missing', sessionKey, 2);

    assert.equal(missing.message, 'Agent session missing; creating fresh session');
    assert.deepEqual(missing.fields, { sessionKey, attempt: 2 });

    const cause = new Error('permission denied');
    const unreadable = sessionLoadDiagnostic('unreadable', sessionKey, 3, cause);

    assert.equal(unreadable.message, 'Agent session unreadable; creating fresh session');
    assert.deepEqual(unreadable.fields, { sessionKey, attempt: 3, error: cause });
});

test('fresh initial prompt includes task prompt and upstream inputs', () => {
    const parts = buildAgentPromptParts({
        prompt: 'Draft the response.',
        workspacePath: '/tmp/relayfold/workflow-1/taskid-draft',
        inputs: [{ customer: 'Ada' }],
        ask: false,
        sessionReused: false,
        approvedTools: [],
        approvedSkills: [],
        canLoadSkills: false,
    });

    assert.match(parts.finalPrompt, /Draft the response/);
    assert.match(parts.finalPrompt, /Upstream task data/);
    assert.match(parts.finalPrompt, /"customer": "Ada"/);
    assert.match(parts.finalPrompt, /WORKSPACE/);
    assert.match(parts.finalPrompt, /\/tmp\/relayfold\/workflow-1\/taskid-draft/);
});

test('loaded human-input continuation includes workspace and submitted response event', () => {
    const parts = buildAgentPromptParts({
        prompt: 'Original prompt should already be in the session.',
        workspacePath: '/tmp/relayfold/workflow-1/taskid-draft',
        inputs: [{ customer: 'Ada' }],
        inputProvided: 'The customer prefers a concise answer.',
        ask: true,
        sessionReused: true,
        approvedTools: [],
        approvedSkills: [],
        canLoadSkills: false,
    });

    assert.match(parts.finalPrompt, /USER RESPONSE TO PREVIOUS INQUIRY/);
    assert.match(parts.finalPrompt, /concise answer/);
    assert.match(parts.finalPrompt, /WORKSPACE/);
    assert.match(parts.finalPrompt, /\/tmp\/relayfold\/workflow-1\/taskid-draft/);
    assert.doesNotMatch(parts.finalPrompt, /Original prompt should already be in the session/);
    assert.doesNotMatch(parts.finalPrompt, /Upstream task data/);
});

test('loaded verifier continuation includes workspace and latest feedback without replaying history', () => {
    const parts = buildAgentPromptParts({
        prompt: 'Original task prompt.',
        workspacePath: '/tmp/relayfold/workflow-1/taskid-draft',
        inputs: [{ source: 'upstream' }],
        loopContext: {
            generation: 3,
            max_iterations: 3,
            feedback_history: [
                { generation: 1, feedback: 'Prior feedback' },
                { generation: 2, feedback: 'Fix the latest issue' },
            ],
            previous_output: { draft: 'previous output' },
        },
        ask: false,
        sessionReused: true,
        approvedTools: [],
        approvedSkills: [],
        canLoadSkills: false,
    });

    assert.match(parts.finalPrompt, /Fix the latest issue/);
    assert.match(parts.finalPrompt, /WORKSPACE/);
    assert.match(parts.finalPrompt, /\/tmp\/relayfold\/workflow-1\/taskid-draft/);
    assert.doesNotMatch(parts.finalPrompt, /Prior verifier feedback history/);
    assert.doesNotMatch(parts.finalPrompt, /Generation 1: Prior feedback/);
    assert.doesNotMatch(parts.finalPrompt, /previous output/);
    assert.doesNotMatch(parts.finalPrompt, /Upstream task data/);
    assert.doesNotMatch(parts.finalPrompt, /Original task prompt/);
});

test('fresh verifier fallback rebuilds full context and current event', () => {
    const parts = buildAgentPromptParts({
        prompt: 'Original task prompt.',
        workspacePath: '/tmp/relayfold/workflow-1/taskid-draft',
        inputs: [{ source: 'upstream' }],
        loopContext: {
            generation: 3,
            max_iterations: 3,
            feedback_history: [
                { generation: 1, feedback: 'Prior feedback' },
                { generation: 2, feedback: 'Fix the latest issue' },
            ],
            previous_output: { draft: 'previous output' },
        },
        ask: false,
        sessionReused: false,
        approvedTools: [],
        approvedSkills: [],
        canLoadSkills: false,
    });

    assert.match(parts.finalPrompt, /Original task prompt/);
    assert.match(parts.finalPrompt, /Upstream task data/);
    assert.match(parts.finalPrompt, /Prior verifier feedback history/);
    assert.match(parts.finalPrompt, /Prior feedback/);
    assert.match(parts.finalPrompt, /previous output/);
    assert.match(parts.finalPrompt, /Fix the latest issue/);
    assert.match(parts.finalPrompt, /WORKSPACE/);
    assert.match(parts.finalPrompt, /\/tmp\/relayfold\/workflow-1\/taskid-draft/);
});

function payload(overrides) {
    return {
        namespace: '550e8400-e29b-41d4-a716-446655440000',
        workflow_inst_id: 'workflow-1',
        task: {
            id: 'draft',
            kind: {
                agent: {
                    model_id: 'test/model',
                    provider_url: '',
                    prompt: 'Draft a response.',
                    tools: [],
                    skills: [],
                    reuse_session: overrides.reuse_session,
                },
            },
            required_credentials: [],
        },
        inputs: [],
        execution_metadata: {
            generation_index: overrides.generation_index,
        },
    };
}

test('system instructions are separate from execution data and rebuilt for restored sessions', () => {
    for (const sessionReused of [false, true]) {
        const args = {
            prompt: 'Investigate the task.',
            workspacePath: '/tmp/task-workspace',
            inputs: [{ evidence: 'upstream-data' }],
            inputProvided: 'user-answer',
            outputSchema: { type: 'object', properties: { result: { type: 'string' } } },
            ask: true,
            allowEarlyExit: true,
            sessionReused,
            approvedTools: [{ name: 'read', description: 'Read approved files' }],
            approvedSkills: [],
            canLoadSkills: true,
        };
        const parts = buildAgentPromptParts(args);
        assert.match(parts.systemContext, /You are an autonomous AI agent/);
        assert.match(parts.systemContext, /read - Read approved files/);
        assert.match(parts.systemContext, /MUST call the 'ask_user' tool/);
        assert.match(parts.systemContext, /EARLY WORKFLOW EXIT/);
        assert.match(parts.systemContext, /Only request early exit when an explicitly stated condition is met/);
        assert.match(parts.systemContext, /If no conditions are provided, continue normal execution/);
        assert.match(parts.systemContext, /FINAL response must be valid JSON/);
        assert.match(parts.systemContext, /"result"/);
        assert.doesNotMatch(parts.systemContext, /Investigate the task|task-workspace|upstream-data|user-answer/);
        assert.doesNotMatch(parts.finalPrompt, /autonomous AI agent|EARLY WORKFLOW EXIT|FINAL response must be valid JSON/);
        assert.match(parts.finalPrompt, /task-workspace/);
        assert.match(parts.finalPrompt, /user-answer/);
        assert.doesNotMatch(buildAgentPromptParts({ ...args, allowEarlyExit: false }).systemContext, /EARLY WORKFLOW EXIT/);
        assert.doesNotMatch(buildAgentPromptParts({ ...args, ask: false }).systemContext, /MUST call the 'ask_user' tool/);
    }
});

test('agent feedback instructions follow current permission for fresh and restored sessions', () => {
    for (const sessionReused of [false, true]) {
        const args = {
            prompt: 'Complete the work.',
            workspacePath: '/tmp/task-workspace',
            inputs: [],
            ask: false,
            sessionReused,
            approvedTools: [],
            approvedSkills: [],
            canLoadSkills: false,
        };
        // Neither an absent nor a permissive schema grants permission.
        for (const outputSchema of [undefined, { type: 'object' }]) {
            assert.doesNotMatch(buildAgentPromptParts({ ...args, outputSchema }).systemContext,
                /AGENT FEEDBACK|_agent_provided_feedback/);
        }
        const enabled = buildAgentPromptParts({ ...args, allowAgentFeedback: true, allowEarlyExit: true });
        assert.match(enabled.systemContext, /AGENT FEEDBACK/);
        assert.match(enabled.systemContext, /environment or setup friction observed/);
        assert.match(enabled.systemContext, /workaround.*concrete improvement/);
        assert.match(enabled.systemContext, /Do not perform additional investigation solely/);
        assert.match(enabled.systemContext, /_agent_provided_feedback string/);
        assert.match(enabled.systemContext, /at most 255 characters/);
        assert.match(enabled.systemContext, /Omit the field or return null/);
        assert.match(enabled.systemContext, /EARLY WORKFLOW EXIT/);
        assert.doesNotMatch(enabled.finalPrompt, /AGENT FEEDBACK|_agent_provided_feedback/);
        const disabled = buildAgentPromptParts({ ...args, allowAgentFeedback: false });
        assert.doesNotMatch(disabled.systemContext, /AGENT FEEDBACK|_agent_provided_feedback/);
    }
});
