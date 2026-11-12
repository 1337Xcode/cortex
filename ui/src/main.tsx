// Entry point: applies the theme, starts loading the fonts, and mounts the app, inside an error
// boundary, into the page's root element.
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { ErrorBoundary } from './ErrorBoundary';
import { loadFonts } from './fonts/fonts';
import './index.css';
import { followSystemTheme } from './theme/applyTheme';

const container = document.getElementById('root');
if (container === null) throw new Error('the page has no #root element');

followSystemTheme(document.documentElement);
void loadFonts();
createRoot(container).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
