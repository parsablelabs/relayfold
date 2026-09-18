---
title: Daily Stock Report Workflow
description: Research stock prices and recent news, assess short-term trajectories, and email an HTML snapshot through Mailgun.
---

[`examples/example_daily_stock_report_workflow.yaml`](https://github.com/parsablelabs/relayfold/blob/main/examples/example_daily_stock_report_workflow.yaml)
is a multi-agent example that builds and emails a dated stock report.

## Example output

[![Example daily stock report for Tesla showing its latest price, recent closes, trajectory, and news.](/relayfold/stock-report-tsla.png)](https://github.com/parsablelabs/relayfold/blob/main/website/public/stock-report-tsla.png)

## Inputs

Start each run with an object containing one ticker symbol and the report
recipient email address:

```json
{
  "ticker": "AAPL",
  "recipient_email": "analyst@example.com"
}
```

## Credentials

Add the model, Twelve Data, and Mailgun credentials to the worker credential
store:

```json
{
  "gemini_api_key": "...",
  "twelve_data": "...",
  "mailgun_api_key": "...",
  "mailgun_domain": "mg.example.com",
  "mailgun_from": "RelayFold Reports <reports@mg.example.com>"
}
```

The Mailgun domain must be authorized to send from the configured address. The
example calls Mailgun's US API endpoint. For an EU-region domain, change the
endpoint in `send-report-email` to `https://api.eu.mailgun.net`.

## Flow

<pre class="mermaid">
flowchart TD
    Input["Ticker + recipient"]
    Quote["fetch-market-quote<br/>API Call task"]
    History["fetch-market-history<br/>API Call task"]
    Market["analyze-market-data<br/>short-term trajectory"]
    News["research-recent-news<br/>last seven days"]
    HTML["compose-html-report<br/>email-safe HTML"]
    Mailgun["send-report-email<br/>Mailgun API"]
    Input --> Quote
    Input --> History
    Quote --> Market
    History --> Market
    Input --> News
    Market --> HTML
    News --> HTML
    HTML --> Mailgun
</pre>

`fetch-market-quote` and `fetch-market-history` are API Call tasks. Each inserts
`${inputs[0].ticker}` into its Twelve Data URL and resolves
`${credentials.twelve_data}` in the authorization header. RelayFold encodes the
ticker before making the requests. `analyze-market-data` combines the raw quote
and time-series responses and describes the observed short-term trajectory. The
news task runs independently and collects up to three recent, sourced items
through browser search.

The Twelve Data Basic (free) plan allows eight API credits per minute. This
demonstration uses two credits per run: one for the quote and one for recent
daily history.

The composition task joins the results by ticker and produces a complete HTML
document with inline styles, source links, and a plain-text fallback. The final
Function task sends both representations through Mailgun. A Mailgun failure
fails the task instead of reporting a successful delivery.

The trajectory is a description of recent historical prices, not investment
advice or a prediction of future performance.

## Register the workflow

The root `fetch-market-quote`, `fetch-market-history`, and
`research-recent-news` tasks each declare the same object input schema.
RelayFold validates the invocation body against those schemas and exposes it to
all three tasks as `inputs[0]`.

```bash
export RELAYFOLD_URL=http://localhost:3000

curl -fsSL https://raw.githubusercontent.com/parsablelabs/relayfold/main/examples/example_daily_stock_report_workflow.yaml \
  | curl -fsS -X POST "$RELAYFOLD_URL/workflow-def" \
      --data-binary @-
```

## Execute the workflow

```bash
curl -fsS -X POST "$RELAYFOLD_URL/workflow-def/daily-stock-report-workflow" \
  -H 'content-type: application/json' \
  -d '{
    "ticker": "AAPL",
    "recipient_email": "analyst@example.com"
  }'
```

## Check the output

After the workflow completes, replace `<workflow_id>` with the `id` returned
when you executed it and read the result of its final task:

```bash
curl -fsS "$RELAYFOLD_URL/workflows/<workflow_id>/tasks/send-report-email"
```
