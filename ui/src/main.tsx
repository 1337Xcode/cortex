// Entry point: applies the theme and mounts the app, inside an error boundary, into the page's root element.
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { ErrorBoundary } from './ErrorBoundary';
import './index.css';
import { followSystemTheme } from './theme/applyTheme';

const container = document.getElementById('root');
if (container === null) throw new Error('the page has no #root element');

followSystemTheme(document.documentElement);
createRoot(container).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
