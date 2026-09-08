import type { ComponentType } from 'react';
import type * as Kit from '../kit';
import type { Theme } from '../theme/ThemeProvider';

export type StoryProps = { theme: Theme };
export interface Story {
  id: string;
  components: readonly (keyof typeof Kit)[];
  size: readonly [width: number, height: number];
  Render: ComponentType<StoryProps>;
}
export function story(
  id: Story['id'],
  components: Story['components'],
  size: Story['size'],
  Render: Story['Render'],
): Story {
  return { id, components, size, Render };
}
