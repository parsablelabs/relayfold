import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import react from '@astrojs/react';

export default defineConfig({
  site: 'https://parsablelabs.github.io',
  base: '/relayfold',
  integrations: [
    react(),
    starlight({
      title: 'RelayFold',
      logo: {
        src: './public/relayfold-logo.png',
        alt: 'RelayFold',
      },
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/parsablelabs/relayfold',
        },
      ],
      components: {
        Head: './src/components/Head.astro',
      },
      sidebar: [
        {
          label: 'Start',
          items: [
            { label: 'Overview', slug: 'docs' },
            { label: 'Install', slug: 'docs/install' },
            { label: 'API Reference', slug: 'docs/api-reference' },
          ],
        },
        {
          label: 'Concepts',
          items: [
            { label: 'Glossary', slug: 'docs/concepts/glossary' },
            { label: 'Workflows', slug: 'docs/concepts/workflows' },
            {
              label: 'Tasks',
              items: [
                { label: 'Overview', slug: 'docs/concepts/tasks' },
                { label: 'Agent Tasks', slug: 'docs/concepts/tasks/agents' },
                { label: 'Function Tasks', slug: 'docs/concepts/tasks/functions' },
                { label: 'API Call Tasks', slug: 'docs/concepts/tasks/api-calls' },
              ],
            },
            { label: 'Bounded Loops', slug: 'docs/concepts/bounded-loops' },
            { label: 'Human Input', slug: 'docs/concepts/human-input' },
            { label: 'Agent Sessions', slug: 'docs/concepts/agent-sessions' },
            { label: 'Workflow Lifecycle', slug: 'docs/concepts/workflow-lifecycle' },
            { label: 'Workflow YAML Reference', slug: 'docs/concepts/workflow-yaml' },
            { label: 'Architecture', slug: 'docs/concepts/architecture' },
          ],
        },
        {
          label: 'Operations',
          items: [
            { label: 'Scheduled Workflows', slug: 'docs/operations/scheduler' },
            { label: 'Scaling', slug: 'docs/operations/scaling' },
            { label: 'Orchestrator Storage', slug: 'docs/operations/storage' },
            { label: 'Workspaces', slug: 'docs/operations/workspaces' },
            { label: 'Credentials', slug: 'docs/operations/credentials' },
            {
              label: 'Production Deployment (WIP)',
              slug: 'docs/operations/production-deployment',
            },
            { label: 'Reliability and Side Effects', slug: 'docs/operations/reliability' },
            { label: 'Worker Host Pinning', slug: 'docs/operations/worker-host-pinning' },
          ],
        },
        {
          label: 'Guides',
          items: [
            { label: 'Register and Run a Workflow', slug: 'docs/guides/register-and-run-workflow' },
            { label: 'Function Registry', slug: 'docs/guides/function-registry' },
            {
              label: 'Safe Agentic Workflows',
              slug: 'docs/guides/safe-agentic-workflows',
            },
          ],
        },
        {
          label: 'Examples',
          items: [
            {
              label: 'Simple Function Workflow',
              slug: 'docs/examples/simple-function-workflow',
            },
            {
              label: 'Human Input Workflow',
              slug: 'docs/examples/human-input-workflow',
            },
            {
              label: 'GitHub Issue to PR',
              slug: 'docs/examples/github-issue-pr-workflow',
            },
            {
              label: 'Daily Stock Report',
              slug: 'docs/examples/daily-stock-report-workflow',
            },
          ],
        },
      ],
    }),
  ],
});
