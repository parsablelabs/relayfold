import type { CredentialsPort } from './ports/CredentialsPort.js';

export class CredentialResolver {
  private static readonly INTERPOLATION_REGEX = /\$\$?\{([^}]+)\}/g;
  private static readonly CREDENTIAL_PATTERN = /^credentials\.([a-zA-Z_][a-zA-Z0-9_]*)$/;

  constructor(private readonly credentialsPort: CredentialsPort) {}

  /**
   * Resolves credential interpolations in a map of headers.
   */
  async resolveHeaders(
    headers: Record<string, string>,
    requiredCredentials: readonly string[]
  ): Promise<Record<string, string>> {
    const resolvedHeaders: Record<string, string> = {};

    for (const [name, value] of Object.entries(headers)) {
      resolvedHeaders[name] = await this.resolveString(value, requiredCredentials);
    }

    return resolvedHeaders;
  }

  /**
   * Resolves credential interpolations in a single string.
   */
  async resolveString(
    value: string,
    requiredCredentials: readonly string[]
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
          throw new Error(`Malformed interpolation expression in header value: unterminated '\${'`);
        }
        const expression = value.substring(i + 2, closingBraceIndex);
        const resolvedValue = await this.resolveExpression(expression, requiredCredentials);
        parts.push(resolvedValue);
        i = closingBraceIndex + 1;
      } else {
        parts.push(value.charAt(i));
        i++;
      }
    }
    return parts.join('');
  }

  private async resolveExpression(
    expression: string,
    requiredCredentials: readonly string[]
  ): Promise<string> {
    const match = expression.match(/^credentials\.([a-zA-Z_][a-zA-Z0-9_]*)$/);
    if (!match) {
      if (expression.startsWith('credentials.')) {
        throw new Error(`Invalid credential name in expression: '\${${expression}}'`);
      }
      const parts = expression.split('.');
      const namespace = parts[0];
      if (namespace !== 'credentials') {
        throw new Error(`Unknown namespace in expression: '\${${expression}}'. Only 'credentials' is supported.`);
      }
      throw new Error(`Malformed expression: '\${${expression}}'`);
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
}
