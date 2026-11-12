// Catches an error thrown while rendering and shows the failed state with its cause, instead of
// leaving the page blank. React offers this only to class components; React itself still reports
// the caught error to the console, so it is shown and never swallowed.
import { Component, type ReactNode } from 'react';

/** The failed state's sentence for an error caught while rendering. */
export function describeRenderFailure(error: unknown): string {
  const cause = error instanceof Error ? error.message : 'an unknown error';
  return `The map stopped with an error (render): ${cause}`;
}

interface BoundaryState {
  readonly error: unknown;
  readonly hasFailed: boolean;
}

/** Shows the page's failed state in place of `children` once any of them throws while rendering. */
export class ErrorBoundary extends Component<{ readonly children: ReactNode }, BoundaryState> {
  override state: BoundaryState = { error: null, hasFailed: false };

  /** Moves the boundary into its failed state. */
  static getDerivedStateFromError(error: unknown): BoundaryState {
    return { error, hasFailed: true };
  }

  override render(): ReactNode {
    if (!this.state.hasFailed) return this.props.children;
    return (
      <main className="page-state">
        <h1>Cortex map</h1>
        <p role="alert">{describeRenderFailure(this.state.error)}</p>
      </main>
    );
  }
}
