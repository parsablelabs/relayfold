---
title: API Call Tasks
description: Use direct API calls for simple HTTP-style workflow steps.
---

API call tasks represent direct service calls in a workflow. Use them when a step can be expressed as a request without model reasoning or custom JavaScript.

```yaml
tasks:
  - id: fetch-status
    kind:
      apiCall:
        url: "https://api.example.com/items"
        method: "GET"
        headers:
          Accept: "application/json"
          Authorization: "Bearer ${credentials.api_token}"
    required_credentials:
      - api_token
```

## When to use API call tasks

Use an API call task when:

- the request shape is simple
- the result can flow directly into downstream data bindings
- the workflow does not need SDK-specific behavior
- a Function task would only wrap one straightforward request

Use a Function task instead when the step needs request signing, provider SDKs, pagination, response normalization, retries with provider-specific behavior, or file output.

## Request contract

An API call supports:

- `url`: the request URL
- `method`: the HTTP method
- `headers`: an optional map of request-header names to string values. Values support [credential interpolation](#credential-interpolation).

Omitting `headers` sends no workflow-configured headers. Request bodies, query-parameter construction, request signing, and task-specific retry behavior are not part of API call tasks; use a Function task when you need those features.

## Credential interpolation

API call tasks support credential interpolation in header values using the `${credentials.<name>}` syntax. This allows authenticated requests without storing secrets in the workflow definition.

```yaml
tasks:
  - id: fetch-items
    kind:
      apiCall:
        url: "https://api.example.com/items"
        method: "GET"
        headers:
          Authorization: "Bearer ${credentials.api_token}"
    required_credentials:
      - api_token
```

### Interpolation rules

- **Syntax**: Expressions use `${credentials.<name>}`. The credential name must start with a letter or underscore and contain only letters, digits, and underscores.
- **Namespacing**: Only the `credentials` namespace is supported.
- **Escaping**: Use `$${` to produce a literal `${` in a header value. For example, `$${credentials.api_token}` becomes `${credentials.api_token}` without being resolved.
- **Validation**: Every referenced credential must be declared in the task's `required_credentials` list.
- **Failure**: The task fails if a referenced credential is undeclared, unavailable, or if the expression is malformed.
- **Single-pass**: Text inserted from a credential is not interpreted again.
- **Security**: Resolved header values and raw credentials are never persisted or included in logs or error messages.

## Response contract

A successful response becomes the complete task output:

```json
{
  "status": 200,
  "headers": {
    "content-type": "application/json"
  },
  "body": {
    "items": []
  }
}
```

Response header names use the normalized form returned by the HTTP runtime. When the response `content-type` is `application/json` or an `application/*+json` media type, `body` is parsed JSON. Other response bodies are strings.

Non-success HTTP statuses, network failures, invalid request configuration, and malformed responses that declare a JSON content type fail the task.

As with other task kinds, declare `input_schemas` and `output_schema` when downstream behavior depends on a specific shape. An API call's `output_schema` describes the complete `{ status, headers, body }` value. RelayFold validates that value without reshaping it before completing the task or passing it downstream.
