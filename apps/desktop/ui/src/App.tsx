import { useEffect, useState } from 'react';
import { BrowserRouter, HashRouter } from 'react-router';
import { DataProvider } from './data/DataProvider';
import type { DataSource } from './data/DataSource';
import { createDataSource } from './data/createDataSource';
import { AppRoutes } from './app/AppRoutes';

export function App() {
  const [source, setSource] = useState<DataSource>();
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let active = true;
    void createDataSource().then(
      (value) => {
        if (active) setSource(value);
      },
      () => {
        if (active) setFailed(true);
      },
    );
    return () => {
      active = false;
    };
  }, []);
  if (failed)
    return (
      <p role="alert" className="xt-startup-status">
        The requested fixture preview could not be loaded. Only F1 is supported.
      </p>
    );
  if (!source)
    return (
      <p role="status" className="xt-startup-status">
        Opening XTrace…
      </p>
    );
  const Router = source.kind === 'native' ? HashRouter : BrowserRouter;
  return (
    <DataProvider source={source}>
      <Router>
        <AppRoutes />
      </Router>
    </DataProvider>
  );
}
