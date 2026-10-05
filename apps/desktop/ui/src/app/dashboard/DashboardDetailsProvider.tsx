import { useState, type ReactNode } from 'react';
import { DetailContext, type DetailKey } from './DashboardDetails';

/**
 * Holds which supplementary measurement is open above the report, so an open
 * detail outlives its trigger while another range loads and reopens over the
 * new report, as the expanded sections it replaces did.
 */
export function DashboardDetailsProvider({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState<DetailKey | null>(null);
  return <DetailContext.Provider value={{ open, setOpen }}>{children}</DetailContext.Provider>;
}
