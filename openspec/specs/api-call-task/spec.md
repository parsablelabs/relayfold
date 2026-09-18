# Capability: api-call-task

## Purpose
Defines how workers resolve and execute direct HTTP requests declared by API Call tasks.

## Requirements

### Requirement: URL Input Interpolation
The worker SHALL resolve input expressions in an API Call task URL before making the network request. Expressions SHALL use `${inputs[index].path}` syntax against the task's resolved input array.

#### Scenario: Nested scalar input
- **WHEN** an API Call URL contains `${inputs[0].account.id}` and that path resolves to a string, number, or boolean
- **THEN** the worker SHALL substitute the value after URL-component encoding it

#### Scenario: Nested array index
- **WHEN** an API Call URL contains `${inputs[0].symbols[1]}` and that path resolves to a scalar value
- **THEN** the worker SHALL substitute the URL-component-encoded second array value

#### Scenario: Missing input value
- **WHEN** a URL input expression references an absent input index or property
- **THEN** the task SHALL fail before making a network request

#### Scenario: Non-scalar input value
- **WHEN** a URL input expression resolves to `null`, an object, or an array
- **THEN** the task SHALL fail before making a network request

#### Scenario: Malformed input expression
- **WHEN** an API Call URL contains an expression outside the supported `inputs[index].path` grammar
- **THEN** the task SHALL fail with a human-readable reason before making a network request

#### Scenario: Escaped expression
- **WHEN** an API Call URL contains `$${inputs[0].value}`
- **THEN** the worker SHALL produce the literal text `${inputs[0].value}` without resolving it

#### Scenario: Input contains an expression
- **WHEN** a resolved input value contains interpolation syntax
- **THEN** the worker SHALL insert it as encoded text without recursively resolving it

### Requirement: Header Credential Interpolation
The worker SHALL resolve `${credentials.name}` expressions in API Call header values from credentials declared in `required_credentials`.

#### Scenario: Declared credential
- **WHEN** a header references an available credential declared in `required_credentials`
- **THEN** the worker SHALL send the resolved value without persisting or logging it

#### Scenario: Credentials in URLs
- **WHEN** an API Call URL references the `credentials` namespace
- **THEN** the task SHALL fail before making a network request
- **THEN** credentials SHALL remain limited to header values

#### Scenario: Inputs in headers
- **WHEN** an API Call header references the `inputs` namespace
- **THEN** the task SHALL fail before making a network request

### Requirement: Request Resolution Failure
The worker SHALL NOT make an API request when URL or header interpolation fails.

#### Scenario: Any request template fails
- **WHEN** either URL input interpolation or header credential interpolation fails
- **THEN** the task SHALL return a human-readable failure that identifies the expression without including credential values
