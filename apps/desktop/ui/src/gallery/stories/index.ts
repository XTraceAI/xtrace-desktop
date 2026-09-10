import { shellStories } from './shell.stories';
import { metricStories } from './metrics.stories';
import { controlStories } from './controls.stories';
import { tableStories } from './tables.stories';
import { overlayStories } from './overlays.stories';
export const stories = [
  ...shellStories,
  ...metricStories,
  ...controlStories,
  ...tableStories,
  ...overlayStories,
];
