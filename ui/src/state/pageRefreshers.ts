// The page's loaders, which a quiet refresh asks again: each endpoint's fetch from the daemon that
// served the page.
import { fetchActivity } from '../api/activity';
import { fetchFileDetail } from '../api/file';
import { fetchGraph } from '../api/graph';
import { fetchHealth } from '../api/health';
import { fetchSummary } from '../api/summary';
import { fetchWarnings } from '../api/warnings';
import type { Refreshers } from './store';

/** Every loader of the page. */
export const PAGE_REFRESHERS: Refreshers = {
  graph: fetchGraph,
  summary: fetchSummary,
  detail: fetchFileDetail,
  activity: fetchActivity,
  warnings: fetchWarnings,
  health: fetchHealth,
};
