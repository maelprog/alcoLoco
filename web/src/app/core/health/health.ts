import { Timestamp } from '../api/timestamp';

/**
 * `GET /health`, typed from `crates/api/src/health.rs`.
 *
 * The endpoint always answers `200`: an unreachable database is reported as
 * data, not as a failure, so `database: "down"` stays distinguishable from a
 * dead process.
 */

/** Path of the endpoint, relative to the API base prefix. */
export const HEALTH_PATH = '/health';

/** Overall verdict. */
export type HealthStatus = 'ok' | 'degraded';

/** State of the database connection behind the API. */
export type DatabaseStatus = 'up' | 'down';

/** Body of `GET /health`. */
export interface HealthResponse {
  /** `ok` when every dependency answers, `degraded` when one does not. */
  status: HealthStatus;
  /** Whether a pooled connection answered the probe query. */
  database: DatabaseStatus;
  /** Instant the probe ran, ISO 8601 UTC. */
  checked_at: Timestamp;
}
