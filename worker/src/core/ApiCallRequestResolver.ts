import type { CredentialsPort } from './ports/CredentialsPort.js';

interface ApiCallRequestTemplate {
  url: string;
  headers?: Record<string, string>;
}

interface ResolvedApiCallRequest {
  url: string;
  headers: Record<string, string>;
}

export class ApiCallRequestResolver {
  private static readonly CREDENTIAL_PATTERN = /^credentials\.([a-zA-Z_][a-zA-Z0-9_]*)$/;
  private static readonly INPUT_PATTERN = /^inputs\[(0|[1-9][0-9]*)\]((?:\.[a-zA-Z_][a-zA-Z0-9_]*|\[(?:0|[1-9][0-9]*)\])*)$/;
  private static readonly INPUT_PATH_SEGMENT_PATTERN = /\.([a-zA-Z_][a-zA-Z0-9_]*)|\[(0|[1-9][0-9]*)\]/g;

  constructor(private readonly credentialsPort: CredentialsPort) {}

  async resolveRequest(
    template: ApiCallRequestTemplate,
    inputs: readonly unknown[],
    requiredCredentials: readonly string[]
  ): Promise<ResolvedApiCallRequest> {
    const url = await this.resolveString(
      template.url,
      'API call URL',
      (expression) => this.resolveInputExpression(expression, inputs)
    );
    const headers: Record<string, string> = {};

    for (const [name, value] of Object.entries(template.headers ?? {})) {
      headers[name] = await this.resolveString(
        value,
        'header value',
        (expression) => this.resolveCredentialExpression(expression, requiredCredentials)
      );
    }

    return { url, headers };
  }

  private async resolveString(
    value: string,
    location: string,
    resolveExpression: (expression: string) => string | Promise<string>
  ): Promise<string> {
    const parts: string[] = [];
    let i = 0;

    while (i < value.length) {
      if (value.startsWith('$${', i)) {
        parts.push('${');
        i += 3;
      } else if (value.startsWith('${', i)) {
        const closingBraceIndex = value.indexOf('}', i + 2);
        if (closingBraceIndex === -1) {
          throw new Error(`Malformed interpolation expression in ${location}: unterminated '\${'`);
        }
        const expression = value.substring(i + 2, closingBraceIndex);
        parts.push(await resolveExpression(expression));
        i = closingBraceIndex + 1;
      } else {
        parts.push(value.charAt(i));
        i++;
      }
    }

    return parts.join('');
  }

  private async resolveCredentialExpression(
    expression: string,
    requiredCredentials: readonly string[]
  ): Promise<string> {
    const match = expression.match(ApiCallRequestResolver.CREDENTIAL_PATTERN);
    if (!match) {
      if (expression.startsWith('credentials.')) {
        throw new Error(`Invalid credential name in expression: '\${${expression}}'`);
      }
      throw new Error(
        `Unknown namespace in expression: '\${${expression}}'. Only 'credentials' is supported in header values.`
      );
    }

    const credentialName = match[1]!;
    if (!requiredCredentials.includes(credentialName)) {
      throw new Error(`Credential '${credentialName}' is referenced but not declared in required_credentials`);
    }

    const value = await this.credentialsPort.getCredential(credentialName);
    if (value === undefined) {
      throw new Error(`Credential '${credentialName}' is unavailable`);
    }

    return value;
  }

  private resolveInputExpression(expression: string, inputs: readonly unknown[]): string {
    const match = expression.match(ApiCallRequestResolver.INPUT_PATTERN);
    if (!match) {
      if (expression.startsWith('inputs')) {
        throw new Error(`Malformed input expression: '\${${expression}}'`);
      }
      throw new Error(
        `Unknown namespace in expression: '\${${expression}}'. Only 'inputs' is supported in API call URLs.`
      );
    }

    const inputIndex = Number(match[1]);
    let value: unknown = this.readPathSegment(inputs, inputIndex, expression);
    const path = match[2] ?? '';

    for (const segment of path.matchAll(ApiCallRequestResolver.INPUT_PATH_SEGMENT_PATTERN)) {
      const key = segment[1] ?? Number(segment[2]);
      value = this.readPathSegment(value, key, expression);
    }

    if (typeof value !== 'string' && typeof value !== 'number' && typeof value !== 'boolean') {
      throw new Error(
        `Input expression '\${${expression}}' must resolve to a string, number, or boolean`
      );
    }

    return encodeURIComponent(String(value));
  }

  private readPathSegment(value: unknown, key: string | number, expression: string): unknown {
    const isExpectedContainer = typeof key === 'number'
      ? Array.isArray(value)
      : value !== null && typeof value === 'object' && !Array.isArray(value);

    if (!isExpectedContainer) {
      throw new Error(`Input expression '\${${expression}}' references a missing value`);
    }

    if (!Object.prototype.hasOwnProperty.call(value, key)) {
      throw new Error(`Input expression '\${${expression}}' references a missing value`);
    }

    return (value as Record<string | number, unknown>)[key];
  }
}
