import type { Report } from './api'

// Retry initial reads on connection errors; stop once a non-active state is known.
export function shouldPollInstance(report: Pick<Report, 'status'> | undefined): boolean {
  return report === undefined || report.status === 'Pending' || report.status === 'Running'
}
