---
status: proposed
---

# Use scheduled PRD delivery campaigns

RelayFold will coordinate a product requirement document (PRD) from a GitHub
issue through decomposition, sequential implementation, and a final pull
request. We will use separate intake and scheduled delivery workflow
definitions, with GitHub as the durable campaign record between workflow
instances, because RelayFold workflow definitions have a static task graph and
workspaces are scoped to one workflow instance.

## Decision

### Intake workflow

A manually started intake workflow will accept a repository and the number of
an issue labeled `PRD`. PRD sequence will me maintained and assigned manually 
with the expectation of unique sequencing.

PRDs will be identified as `PRD NNNN`, and their child issues as
`NNNN-NNNN`. The PRD title will begin with `[PRD NNNN]`; child titles will
begin with `[PRD NNNN-NNNN]`. A unique `prd:NNNN` label will be applied to the
PRD and all of its children so the entire campaign can be queried without
depending on title parsing.

An Agent task may propose the decomposition and acceptance criteria, but
schema-constrained Function tasks will re-read GitHub state and perform the
number reservation, label changes, and issue creation. This keeps generated
content separate from the authority to mutate the repository.

### Scheduled delivery workflow

One scheduler entry will periodically start a dispatcher workflow for the
repository. The dispatcher will select the lowest-numbered unfinished PRD child issue to work on. After work is done, next scheduled dispatche will find new lowest sequence for child issue to work on.

Each scheduled workflow instance will advance at most one child issue. It will:

1. Re-read the PRD, its addenda, and every issue carrying the campaign label.
2. Select the lowest-numbered queued child.
3. Fetch or create the remote branch `relayfold/prd-NNNN` in the workflow
   instance's workspace and synchronize it with the repository's default
   branch.
4. Implement the child issue and run focused tests.
5. Use a bounded verifier loop to review the change against the child issue,
   PRD, and applicable addenda.
6. After acceptance, commit and push the change, comment on the child with the
   commit identifier, and mark the child implemented without closing it.

The remote campaign branch, rather than a RelayFold workspace or Agent session,
is the durable source for code changes across scheduled workflow instances.
This permits later instances to run on another eligible worker host.

GitHub labels will represent campaign and child lifecycle states such as
queued, active, implemented, obsolete, blocked, and PR open. The exact label
names and issue-body markers belong to the workflow contract, not to the
RelayFold runtime API.

### Requirements reconciliation

The implementation task will report discoveries that materially change the
remaining requirements. After verifier acceptance, a reconciliation step may:

- append a dated addendum to the PRD that identifies the triggering child and
  explains the change;
- revise queued child issues;
- add new children using the NNNN-NNNN-N numberinc scheme, e.g. 0001-0010-1; or
- mark queued children obsolete without reusing their numbers by adding another label "cancelled" which next iteration should ignore.
- preference should be to adjust existing issue, instead of redoing the issues and sequences.

The original PRD text and **completed** child issues will not be rewritten.
Equivalent addenda and issue edits will carry stable markers so a retry is a
no-op. Obsolete children will receive an explanatory comment before they are
closed.

### Final audit and pull request

When no queued children remain, a scheduled workflow instance will skip over and audit the
complete branch against the PRD and all addenda and run the campaign-level test
suite. If the audit finds missing work, the workflow will create new numbered
children and defer pull-request creation to a later scheduled run.

When the audit passes, a narrow mutation task will create one pull request from
`relayfold/prd-NNNN`. Before creating it, the task will look for an existing
pull request for the branch so retrying finalization cannot create duplicates.
The pull request will summarize the PRD, addenda, child issues, and tests, and
will contain closing references for the PRD and all implemented children. The
PRD and implemented children therefore close when the pull request is merged,
not when implementation is merely pushed.

### Failure handling

All GitHub mutations and branch checkpoints must be safe for at-least-once
task execution. Mutation tasks will re-read current state, validate that the
target belongs to the selected repository and campaign, and treat an already
applied operation as success.

Ambiguous requirements, branch synchronization conflicts, and decisions
outside the workflow's declared authority will move the active task to
`InputNeeded`. Known unrecoverable campaign conditions will be recorded on the
PRD and marked blocked. A blocked or input-needed campaign will not be skipped
silently in favor of later work; an operator must resolve or explicitly
deprioritize it.

## Considered options

- **One long-lived workflow instance for the entire PRD.** Rejected because
  the number of child issues is not known when the static workflow definition
  is registered, and keeping a workflow instance active for the lifetime of a
  large campaign makes recovery and operator intervention harder.
- **One scheduler entry per PRD using the same workflow definition.** Rejected
  because scheduler overlap prevention is keyed by namespace and workflow
  definition, not trigger input; competing entries could repeatedly prevent a
  later PRD from running.
- **Use the workflow workspace as cross-run campaign state.** Rejected because
  workspaces belong to workflow instances and execution hosts. GitHub issues
  and a remote branch are already durable, observable, and accessible from any
  eligible worker host.
- **Create one pull request per child issue.** Rejected because the requested
  delivery unit is the complete PRD. Per-child commits retain traceability
  while one final pull request presents and validates the integrated change.

## Consequences

Campaign progress remains visible and recoverable in GitHub even if RelayFold
is restarted or a later workflow instance uses another worker host. Limiting a
scheduled instance to one child bounds execution time and lets the scheduler
provide continuous progress without requiring dynamic workflow tasks.

The design introduces two workflow definitions and several idempotent GitHub
mutation Functions instead of one self-contained workflow. GitHub label and
issue conventions become part of the operational contract, and the campaign
branch may live for multiple scheduler intervals. Because work is serialized
per repository, a blocked active campaign intentionally stops later campaigns
until an operator acts.

This decision defines how a workflow can be built from existing RelayFold
capabilities. It does not add scheduler semantics, dynamic task generation, or
new public RelayFold APIs.
