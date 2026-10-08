import type { Report, Task } from './api'

export function latestTaskAttempts(attempts: Task[]): Task[] {
  const latest = new Map<string, Task>()
  for (const attempt of attempts) {
    const previous = latest.get(attempt.task_def_id)
    if (!previous || attempt.generation_index > previous.generation_index) latest.set(attempt.task_def_id, attempt)
  }
  return [...latest.values()]
}

export function pendingHumanInputs(report: Pick<Report, 'status' | 'tasks'>): Task[] {
  if (report.status !== 'InputNeeded') return []
  return latestTaskAttempts(report.tasks).filter(task => typeof task.status === 'object')
}

// Retry initial reads on connection errors; stop once a non-active state is known.
export function shouldPollInstance(report: Pick<Report, 'status'> | undefined): boolean {
  return report === undefined || report.status === 'Pending' || report.status === 'Running'
}
