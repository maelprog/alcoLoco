import { Component, input } from '@angular/core';

/**
 * One event: its participants, their current alcohol level and their compared
 * curves (SPEC.md §5.2, §5.7).
 *
 * The identifier comes from the route through component input binding, so the
 * screen never reads `ActivatedRoute` itself.
 */
@Component({
  selector: 'app-event-detail-page',
  template: `
    <h2>Événement</h2>
    <p>Identifiant&nbsp;: {{ eventId() }}</p>
    <p>Participants, alcoolémies courantes et courbes comparées.</p>
  `,
})
export class EventDetailPage {
  /** Identifier of the event, bound from the `:eventId` path parameter. */
  readonly eventId = input.required<string>();
}
