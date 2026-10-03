import type { TaskExecutor, JsonValue, TaskExecutionResult } from '../../core/ports/TaskExecutor.js';
import type { TaskExecutionPayload } from '../../core/models/TaskDef.js';
import type { CredentialsPort } from '../../core/ports/CredentialsPort.js';
import { getModel } from '@earendil-works/pi-ai';
import { Agent } from '@earendil-works/pi-agent-core';
import { AuthStorage, createAgentSession, createCodingTools, DefaultResourceLoader, ModelRegistry, SettingsManager, formatSkillsForPrompt, SessionManager, type Skill } from '@earendil-works/pi-coding-agent';
import { logger } from '../../utils/logger.js';
import { createJsonSchemaValidator } from '../../core/JsonSchemaValidator.js';
import { createBraveSearchTool } from './agent_tools/braveSearchTool.js';
import { createFetchUrlTool } from './agent_tools/fetchUrlTool.js';
import { createHttpRequestTool } from './agent_tools/httpRequestTool.js';
import { createCurrentTimeTool } from './agent_tools/currentTimeTool.js';
import { createAskUserTool, InputNeededError } from './agent_tools/askUserTool.js';
import { ToolRegistry } from './agent_tools/ToolRegistry.js';
import { PiResourceToolProvider } from './agent_tools/PiResourceToolProvider.js';
import { selectAgentTools } from './agent_tools/toolSelection.js';
import { selectApprovedSkills } from './agent_tools/skillSelection.js';
import type { SessionStore } from '../../core/ports/SessionStore.js';
import { nativeSessionDir, persistSessionBestEffort, materializePiSessionFile } from '../../core/ports/SessionStore.js';
import { agentSessionKey } from '../../core/models/AgentSession.js';
import { resolveCredentialEnvironment, resolveTaskProviderApiKey, withTaskEnvironment } from '../../core/TaskEnvironment.js';
import type { TaskSandbox } from '../../core/ports/TaskSandbox.js';
import { createSandboxCodingTools } from './agent_tools/sandboxCodingTools.js';

function extractAssistantText(agent: Agent): string {
    let resultText = '';
    const lastMsg = agent.state.messages[agent.state.messages.length - 1];
    if (lastMsg && lastMsg.role === 'assistant') {
        for (const content of lastMsg.content) {
            if (content.type === 'text') {
                resultText += content.text;
            }
        }
    }
    return resultText;
}

export function extractJsonString(resultText: string): string {
    let jsonString = resultText.trim();

    if (!jsonString) {
        return "";
    }

    // Prefer the complete response when it is already valid JSON. This preserves
    // Markdown fences and other brace-like content inside JSON string values.
    try {
        JSON.parse(jsonString);
        return jsonString;
    } catch {
        // Continue with compatibility extraction for fenced or prefixed output.
    }

    // Remove a Markdown code block only when it wraps the complete response.
    const markdownMatch = jsonString.match(/^```(?:json)?\s*([\s\S]*?)\s*```\s*$/i);
    if (markdownMatch?.[1]) {
        jsonString = markdownMatch[1].trim();
    }

    const firstObjectBrace = jsonString.indexOf('{');
    const firstArrayBracket = jsonString.indexOf('[');
    const hasObject = firstObjectBrace !== -1;
    const hasArray = firstArrayBracket !== -1;

    if (!hasObject && !hasArray) {
        return ""; // No JSON structures found
    }

    const startsWithArray = hasArray && (!hasObject || firstArrayBracket < firstObjectBrace);
    const firstJsonChar = startsWithArray ? firstArrayBracket : firstObjectBrace;
    const lastJsonChar = startsWithArray ? jsonString.lastIndexOf(']') : jsonString.lastIndexOf('}');

    if (firstJsonChar !== -1 && lastJsonChar !== -1 && lastJsonChar > firstJsonChar) {
        return jsonString.substring(firstJsonChar, lastJsonChar + 1).trim();
    }
    return "";
}

/**
 * Attempts to repair common JSON errors like unescaped quotes or missing braces.
 */
function repairJson(str: string): string {
    let repaired = str.trim();
    if (!repaired) return "";

    // If it doesn't start with { or [, it's definitely not JSON
    if (!repaired.startsWith('{') && !repaired.startsWith('[')) return "";

    try {
        JSON.parse(repaired);
        return repaired; // Already valid
    } catch (e) {
        // Try a very basic "loose" repair for unescaped quotes in string values:
        // This regex looks for quotes that are NOT preceded by : or , or { or [
        // and NOT followed by : or , or } or ]
        // Note: This is a heuristic and not perfect.
        try {
            const partiallyRepaired = repaired.replace(/([^\s:{[,])"([^\s:}\],])/g, '$1\\"$2');
            JSON.parse(partiallyRepaired);
            return partiallyRepaired;
        } catch (e2) {
            return repaired; // Give up and return original for standard error handling
        }
    }
}

function parseRetryTimes(value: unknown): number {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
        return 0;
    }
    return Math.max(0, Math.floor(value));
}

export function shouldReuseAgentSession(payload: TaskExecutionPayload): boolean {
    return 'agent' in payload.task.kind && payload.task.kind.agent.reuse_session !== false;
}

export function shouldLoadExistingAgentSession(payload: TaskExecutionPayload): boolean {
    const attempt = payload.execution_metadata?.generation_index ?? 1;
    return shouldReuseAgentSession(payload) && attempt > 1;
}

export function sessionLoadDiagnostic(
    problem: 'missing' | 'unreadable',
    sessionKey: unknown,
    attempt: number,
    error?: unknown
): { fields: Record<string, unknown>; message: string } {
    return {
        fields: problem === 'unreadable'
            ? { sessionKey, attempt, error }
            : { sessionKey, attempt },
        message: problem === 'unreadable'
            ? 'Agent session unreadable; creating fresh session'
            : 'Agent session missing; creating fresh session',
    };
}

type PromptTool = {
    name: string;
    description: string;
};

type ExecutionLoopContext = NonNullable<NonNullable<TaskExecutionPayload['execution_metadata']>['loop_context']>;

export type AgentPromptParts = {
    systemContext: string;
    freshContext: string;
    currentEventContext: string;
    finalPrompt: string;
};

export function buildAgentPromptParts(args: {
    prompt: string;
    workspacePath: string;
    inputs: unknown[];
    loopContext: ExecutionLoopContext | undefined;
    inputProvided: string | undefined;
    outputSchema: unknown | undefined;
    ask: boolean;
    sessionReused: boolean;
    approvedTools: PromptTool[];
    approvedSkills: Skill[];
    canLoadSkills: boolean;
}): AgentPromptParts {
    let systemContext = "";
    let freshContext = "";
    let currentEventContext = "";

    currentEventContext += `
                \n\n
                WORKSPACE:
                Use this selected workspace path for task file work:
                ${args.workspacePath}
                `;

    if (args.inputs.length > 0) {
        freshContext += `\n\nUpstream task data:\n`;
        for (const input of args.inputs) {
            freshContext += `${JSON.stringify(input, null, 2)}\n`;
        }
    }

    const loopContext = args.loopContext;
    const latestFeedback = loopContext?.feedback_history?.at(-1)?.feedback;
    const feedbackHistory = loopContext?.feedback_history?.slice(0, -1) || [];

    if (latestFeedback || loopContext?.previous_output !== undefined) {
        currentEventContext += `
                \n\n
                VERIFIER-GUIDED RETRY:
                This execution includes verifier feedback from a previous rejected generation.
                Treat that feedback as orchestration guidance for revising your result.
                The most recent feedback identifies the current issue to fix.
                Prior feedback records earlier corrections to preserve when applicable.
                Do not include the feedback text in the final output unless the task explicitly asks for it.\n`;

        if (latestFeedback) {
            currentEventContext += `
                    \n\n
                    **Most recent verifier feedback:**\n
                    ${latestFeedback}\n`;
        }

        if (!args.sessionReused && feedbackHistory.length > 0) {
            currentEventContext += `
                    \n\n
                    **Prior verifier feedback history:**\n`;

            feedbackHistory.forEach((entry) => {
                currentEventContext += `- Generation ${entry.generation}: ${entry.feedback}\n`;
            });
        }

        if (!args.sessionReused && loopContext?.previous_output !== undefined) {
            currentEventContext += `
                    \n\n
                    **Most recent previous output from this same task:**\n
                    ${JSON.stringify(loopContext.previous_output, null, 2)}\n`;
        }
    }

    if (args.inputProvided) {
        currentEventContext += `\n\nUSER RESPONSE TO PREVIOUS INQUIRY:\n${args.inputProvided}\n`;
    }

    if (!args.sessionReused) {
        const toolAvailabilityPrompt = args.approvedTools.length > 0
            ? `You have access to the following approved tools:\n${args.approvedTools.map((approvedTool) => `- ${approvedTool.name} - ${approvedTool.description}`).join('\n')}`
            : "You do not have access to any tools for this task.";

        const skillsPrompt = args.approvedSkills.length > 0 && args.canLoadSkills
            ? `\n\n${formatSkillsForPrompt(args.approvedSkills)}`
            : '';

        systemContext += `
                ${args.ask ? "CRITICAL: If you cannot complete the task because you need more information or clarification from the user, you MUST call the 'ask_user' tool. DO NOT return a JSON object with a response that asks a question. Calling 'ask_user' is the ONLY way to request more information." : ""}

                You are an autonomous AI agent.
                ${toolAvailabilityPrompt}
                ${skillsPrompt}

                NOTE: You are allowed and encouraged to use approved tools to gather information FIRST before producing the final response. 
                DO NOT ask for permission before using your approved tools. Execute them right away and use the results to answer the user.
                `;
    }

    if (args.outputSchema) {
        currentEventContext += `
            \n\n
            IMPORTANT: Your FINAL response must be valid JSON that adheres to the following schema:
            \n
            ${JSON.stringify(args.outputSchema, null, 2)}

            CRITICAL: When you have gathered all information and are ready to provide the final answer, you MUST output ONLY the raw JSON object. 
            - Do NOT include any conversational text or explanations.
            - Do NOT include any preamble like "Here is the result".
            - Do NOT wrap the JSON in markdown code blocks (e.g., no \`\`\`json).
            - The entire response must be parseable by JSON.parse().
            If output_schema validation fails, you will be asked to correct the JSON.
            ${args.ask ? "REMINDER: If you are missing information to fulfill the request, use the 'ask_user' tool (if enabled) instead of returning JSON." : ""}`;
    }

    return {
        systemContext,
        freshContext,
        currentEventContext,
        finalPrompt: args.sessionReused
            ? currentEventContext
            : systemContext + args.prompt + freshContext + currentEventContext,
    };
}

export class AgentExecutor implements TaskExecutor {
    async execute(payload: TaskExecutionPayload, credentialsPort: CredentialsPort, sessionStore: SessionStore, sandbox?: TaskSandbox): Promise<TaskExecutionResult> {
        const agentDef = (payload.task.kind as any).agent;
        const modelIdFull = agentDef.model_id as string;

        if (!modelIdFull.includes('/')) {
            throw new Error(`Invalid model_id format: '${modelIdFull}'. Expected format 'provider/model' (e.g., 'google/gemini-2.5-flash').`);
        }

        const envCredentials = await resolveCredentialEnvironment(payload, credentialsPort);
        if (sandbox) {
            const apiKey = resolveTaskProviderApiKey(modelIdFull.split('/')[0]!, envCredentials);
            return this.executeWithEnvironment(payload, credentialsPort, sessionStore, sandbox, apiKey);
        }
        return await withTaskEnvironment(
            envCredentials,
            () => this.executeWithEnvironment(payload, credentialsPort, sessionStore, sandbox)
        );
    }

    private async executeWithEnvironment(
        payload: TaskExecutionPayload,
        credentialsPort: CredentialsPort,
        sessionStore: SessionStore,
        sandbox?: TaskSandbox,
        apiKey?: string
    ): Promise<TaskExecutionResult> {
        const agentDef = (payload.task.kind as any).agent;
        const ask = (agentDef.ask ?? (payload.task as any).ask) === true;
        const modelIdFull = agentDef.model_id as string;
        const providerUrl = agentDef.provider_url as string;
        const prompt = agentDef.prompt as string;

        logger.info(`[AgentExecutor] Running agent model: ${modelIdFull} with provider: ${providerUrl}`);

        // START - Configure Agent
        const parts = modelIdFull.split('/');
        const providerName = parts[0];
        const modelName = parts.slice(1).join('/');
        if (!providerName) {
            throw new Error(`Invalid model_id format: '${modelIdFull}'. Expected a non-empty provider name.`);
        }

        let model = getModel(providerName as any, modelName as any);
        if (!model) {
            throw new Error(`Model not found for ${providerName}/${modelName}`);
        }

        // Override the baseUrl with the one provided in the task definition
        if (providerUrl) {
            model = { ...model, baseUrl: providerUrl };
        }

        let sessionKey = agentSessionKey(payload);
        let sessionReused = false;
        let sessionManager = null;
        let reuseSession = shouldReuseAgentSession(payload);

        if (reuseSession) {
            let attempt = payload.execution_metadata?.generation_index ?? 1;

            if (shouldLoadExistingAgentSession(payload)) {
                try {
                    let session_data = await sessionStore.load(sessionKey);
                    if (session_data !== null) {
                        sessionReused = true;
                        const tempSessionFilePath = await materializePiSessionFile(sessionKey, session_data.content);
                        sessionManager = SessionManager.open(tempSessionFilePath);
                        logger.info(
                            { sessionKey, attempt },
                            "Loaded existing agent session"
                        )
                    } else {
                        const diagnostic = sessionLoadDiagnostic('missing', sessionKey, attempt);
                        logger.warn(diagnostic.fields, diagnostic.message)
                    }

                } catch (error: unknown) {
                    const diagnostic = sessionLoadDiagnostic('unreadable', sessionKey, attempt, error);
                    logger.warn(diagnostic.fields, diagnostic.message);
                }
            }
        }
        
        if (sessionManager == null) {
            logger.info("Creating new agent session");
            sessionManager = SessionManager.create(process.cwd(), nativeSessionDir())
        }

        // Consider moving tool registration in here
        const resourceLoader = sandbox ? new DefaultResourceLoader({
            agentDir: nativeSessionDir(),
            cwd: process.cwd(), settingsManager: SettingsManager.inMemory(),
            noExtensions: true, noSkills: true, noContextFiles: true,
            noPromptTemplates: true, noThemes: true,
        }) : undefined;
        await resourceLoader?.reload();
        const authStorage = AuthStorage.inMemory();
        if (apiKey) authStorage.setRuntimeApiKey(providerName, apiKey);
        const { session } = await createAgentSession({
            ...(sandbox && resourceLoader ? {
                tools: [],
                resourceLoader,
                modelRegistry: ModelRegistry.inMemory(authStorage),
                cwd: sandbox.workspacePath,
                settingsManager: SettingsManager.inMemory(),
            } : {}),
            sessionManager,
            model,
            authStorage,
        });

        const agent = session.agent;

        // Tool use event handling
        agent.subscribe((event) => {
            if (event.type === 'tool_execution_start') {
                logger.info({ toolName: event.toolName, args: event.args }, '[AgentExecutor] Agent tool started');
            } else if (event.type === 'tool_execution_end') {
                logger.info(
                    { toolName: event.toolName, isError: event.isError },
                    '[AgentExecutor] Agent tool finished'
                );
            }
        });

        // Tool setup
        const toolRegistry = new ToolRegistry();
        toolRegistry.registerTools([
            createFetchUrlTool(sandbox?.fetch),
            createHttpRequestTool(sandbox?.fetch),
            createCurrentTimeTool(),
        ]);
        toolRegistry.registerTools(sandbox ? createSandboxCodingTools(sandbox) : createCodingTools(process.cwd()));
        const piResources = sandbox ? { tools: [], skills: sandbox.skills } : await new PiResourceToolProvider().loadResources();
        toolRegistry.registerTools(piResources.tools);

        const systemBraveApiKey = "system_brave_api_key";
        const braveApiKey = !sandbox || payload.task.required_credentials.includes(systemBraveApiKey)
            ? await credentialsPort.getCredential(systemBraveApiKey) : undefined;
        if (braveApiKey) {
            toolRegistry.registerTool(createBraveSearchTool(braveApiKey, sandbox?.fetch));
        } else {
            logger.error(`Error retrieving system credentials for ${systemBraveApiKey}`);
        }

        let inputNeededQuestion: string | undefined = undefined;

        const askUserTool = ask
            ? createAskUserTool((question) => {
                inputNeededQuestion = question;
                agent.abort();
            })
            : undefined;

        const availableTools = toolRegistry.getTools();
        const { approvedTools, unavailableApprovedToolNames } = selectAgentTools(
            availableTools,
            agentDef.tools,
            askUserTool
                ? {
                    name: askUserTool.name,
                    description: askUserTool.description,
                    tool: askUserTool,
                }
                : undefined,
        );
        const { approvedSkills, unavailableApprovedSkillNames } = selectApprovedSkills(piResources.skills, agentDef.skills);

        if (unavailableApprovedToolNames.length > 0) {
            if (sandbox) throw new Error(`Tools are not supported in sandboxed tasks: ${unavailableApprovedToolNames.join(', ')}`);
            logger.warn({ unavailableApprovedToolNames }, "[AgentExecutor] Ignoring approved tools that are not available");
        }

        if (unavailableApprovedSkillNames.length > 0) {
            throw new Error(`Requested agent skills are not available: ${unavailableApprovedSkillNames.join(', ')}`);
        }

        const canLoadSkills = approvedTools.some((approvedTool) => approvedTool.name === 'read');
        if (approvedSkills.length > 0 && !canLoadSkills) {
            throw new Error('Agent skills require the read tool so the agent can load SKILL.md content');
        }
        agent.state.tools = approvedTools.map((approvedTool) => approvedTool.tool);
        // END - Configure Agent

        const loopContext = payload.execution_metadata?.loop_context;
        if (loopContext?.feedback_history?.at(-1)?.feedback || loopContext?.previous_output !== undefined) {
            logger.info('[AgentExecutor] Verifier feedback provided')
        }

        if (payload.input_provided) {
            logger.info('[AgentExecutor] User response provided')
        }

        const { finalPrompt } = buildAgentPromptParts({
            prompt,
            workspacePath: payload.workspace_path,
            inputs: payload.inputs,
            loopContext,
            inputProvided: payload.input_provided,
            outputSchema: payload.task.output_schema,
            ask,
            sessionReused,
            approvedTools,
            approvedSkills,
            canLoadSkills,
        });

        logger.info(
            {
                userPrompt: finalPrompt,
                systemPromptLength: agent.state.systemPrompt.length,
            },
            "[AgentExecutor] Final agent prompt"
        );

        const abortOnClose = () => agent.abort();
        sandbox?.signal.addEventListener('abort', abortOnClose, { once: true });
        try {
            if (sandbox?.signal.aborted) throw new Error('Task sandbox is closed');
            await session.prompt(finalPrompt);

            if (inputNeededQuestion) {
                return { status: 'input_needed', description: inputNeededQuestion };
            }

            if (agent.state.errorMessage) {
                throw new Error(`Agent failed: ${agent.state.errorMessage}`);
            }

            if (payload.task.output_schema) {
                const retryTimes = parseRetryTimes(agentDef.schema_failure_retry_times);
                const ajv = createJsonSchemaValidator();
                const validate = ajv.compile(payload.task.output_schema);

                for (let attempt = 0; attempt <= retryTimes; attempt++) {
                    const resultText = extractAssistantText(agent);
                    let parsed: JsonValue = null;
                    let parseErrorMessage: string | undefined;

                    const rawExtracted = extractJsonString(resultText);
                    const repaired = repairJson(rawExtracted);

                    try {
                        if (repaired) {
                            parsed = JSON.parse(repaired);
                        } else if (payload.task.output_schema?.properties?.response) {
                            // FALLBACK: If no JSON found but we expect a 'response' string, wrap the raw text
                            logger.info("[AgentExecutor] No JSON found, applying auto-wrap fallback");
                            parsed = { response: resultText.trim() };
                        } else {
                            throw new Error("No JSON object found in response");
                        }
                    } catch (err) {
                        parseErrorMessage = err instanceof Error ? err.message : String(err);
                    }

                    if (parseErrorMessage === undefined) {
                        if (validate(parsed)) {
                            return await finalizeAgentOutput(payload, parsed);
                        }

                        const validationMessage = ajv.errorsText(validate.errors);
                        if (attempt >= retryTimes) {
                            logger.error({ validationMessage, rawResponse: resultText }, "[AgentExecutor] JSON response failed output_schema validation");
                            throw new Error(`Agent JSON response failed output_schema validation: ${validationMessage}`);
                        }

                        await session.prompt(`
                            Your previous response failed output_schema validation:
                            ${validationMessage}

                            Return ONLY a corrected raw JSON object that satisfies the schema.
                        `);
                    } else {
                        if (attempt >= retryTimes) {
                            logger.error({ parseErrorMessage, rawResponse: resultText }, "[AgentExecutor] Failed to parse expected JSON");
                            throw new Error(`Failed to parse JSON response from agent. Expected schema: ${JSON.stringify(payload.task.output_schema)}`);
                        }

                        await session.prompt(`
                            Your previous response was not valid parseable JSON:
                            ${parseErrorMessage}

                            Return ONLY a corrected raw JSON object that satisfies the schema.
                        `);
                    }
                }
            }
        } catch (e) {
            logger.error({ error: e, message: (e as any).message, stack: (e as any).stack }, "[AgentExecutor] Error during agent execution");
            if (e instanceof InputNeededError || inputNeededQuestion) {
                return { status: 'input_needed', description: inputNeededQuestion || (e as any).question || "Input needed" };
            }
            throw e;
        } finally {
            sandbox?.signal.removeEventListener('abort', abortOnClose);
            if (reuseSession) {
                await persistSessionBestEffort(sessionKey, session, sessionStore);
            }
        }

        const resultText = extractAssistantText(agent);
        return await finalizeAgentOutput(payload, { response: resultText });
    }
}

async function finalizeAgentOutput(
    _payload: TaskExecutionPayload,
    output: JsonValue
): Promise<TaskExecutionResult> {
    return { status: 'ok', output };
}
