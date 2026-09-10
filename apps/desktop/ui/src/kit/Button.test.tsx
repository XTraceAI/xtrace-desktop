import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { Button } from './Button';
afterEach(cleanup);

it('supports variants/heights and blocks disabled native actions', () => {
  const action = vi.fn();
  render(
    <>
      <Button variant="accent" height={38} onClick={action}>
        Save
      </Button>
      <Button variant="outline" disabled onClick={action}>
        Unavailable
      </Button>
    </>,
  );
  const save = screen.getByRole('button', { name: 'Save' });
  expect(save.getAttribute('type')).toBe('button');
  expect(save.style.height).toBe('38px');
  expect(save.getAttribute('data-variant')).toBe('accent');
  fireEvent.click(save);
  fireEvent.click(screen.getByText('Unavailable'));
  expect(action).toHaveBeenCalledTimes(1);
});
