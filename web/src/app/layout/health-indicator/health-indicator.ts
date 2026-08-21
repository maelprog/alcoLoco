import { DatePipe } from '@angular/common';
import { Component, inject, signal } from '@angular/core';

import { ApiError, userMessage } from '../../core/api/api-error';
import { HealthClient } from '../../core/health/health-client';
import { DatabaseStatus, HealthResponse, HealthStatus } from '../../core/health/health';

/**
 * Development banner reporting what `GET /health` answers.
 *
 * It is mounted by the shell only when {@link DEV_MODE} holds, so that a
 * developer sees at a glance whether the API and its database are reachable
 * behind the dev server proxy, without opening the network panel.
 *
 * The timestamp is rendered through `DatePipe`, which converts the UTC instant
 * the API sends into the reader’s time zone — the conversion the API refuses to
 * do (SPEC.md §7).
 */
@Component({
  selector: 'app-health-indicator',
  imports: [DatePipe],
  template: `
    <aside class="health" [attr.data-state]="state()">
      <span class="health__dot" aria-hidden="true"></span>
      @if (failure(); as message) {
        <span>API&nbsp;: {{ message }}</span>
      } @else if (health(); as report) {
        <span>
          API&nbsp;: {{ statusLabel(report.status) }} — base&nbsp;:
          {{ databaseLabel(report.database) }} — vérifié à
          {{ report.checked_at | date: 'HH:mm:ss' }}
        </span>
      } @else {
        <span>API&nbsp;: vérification en cours…</span>
      }
    </aside>
  `,
  styles: `
    .health {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.375rem 0.75rem;
      font-size: 0.8125rem;
      background: #f4f4f5;
      border-top: 1px solid #d4d4d8;
      color: #3f3f46;
    }

    .health__dot {
      width: 0.5rem;
      height: 0.5rem;
      border-radius: 50%;
      background: #a1a1aa;
    }

    .health[data-state='ok'] .health__dot {
      background: #16a34a;
    }

    .health[data-state='degraded'] .health__dot {
      background: #ea580c;
    }

    .health[data-state='unreachable'] .health__dot {
      background: #dc2626;
    }
  `,
})
export class HealthIndicator {
  private readonly client = inject(HealthClient);

  /** The last answer of `GET /health`, `null` until one arrives. */
  protected readonly health = signal<HealthResponse | null>(null);

  /** The message to show when the call failed, `null` otherwise. */
  protected readonly failure = signal<string | null>(null);

  constructor() {
    this.client.check().subscribe({
      next: (report) => this.health.set(report),
      error: (error: unknown) => {
        this.failure.set(
          error instanceof ApiError ? userMessage(error) : 'Une erreur inattendue est survenue.',
        );
      },
    });
  }

  /** Drives the colour of the dot: `pending`, `ok`, `degraded`, `unreachable`. */
  protected state(): string {
    if (this.failure() !== null) {
      return 'unreachable';
    }
    return this.health()?.status ?? 'pending';
  }

  protected statusLabel(status: HealthStatus): string {
    return status === 'ok' ? 'en ligne' : 'dégradée';
  }

  protected databaseLabel(database: DatabaseStatus): string {
    return database === 'up' ? 'connectée' : 'injoignable';
  }
}
