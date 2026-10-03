import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { applicationVersion } from '../applicationVersion';
import hostSidebarSource from './HostSidebar.tsx?raw';
import { HostSidebar } from './HostSidebar';

vi.mock('../api/queries', () => ({ useHostSession: vi.fn() }));

function renderSidebar(applicationVersion?: string) {
  render(
    <HostSidebar
      hosts={[]}
      selectedId={null}
      onSelect={vi.fn()}
      onAdd={vi.fn()}
      activeSection="overview"
      onSection={vi.fn()}
      applicationVersion={applicationVersion}
    />,
  );
}

describe('application version label', () => {
  it('uses the release-verified build version by default', () => {
    renderSidebar();

    expect(screen.getByText(`v${applicationVersion}`)).toBeInTheDocument();
  });

  it('renders the full supplied semantic version', () => {
    renderSidebar('9.8.7');

    expect(screen.getByText('v9.8.7')).toBeInTheDocument();
  });

  it('preserves prerelease and build metadata', () => {
    renderSidebar('9.8.7-beta.1+build.5');

    expect(screen.getByText('v9.8.7-beta.1+build.5')).toBeInTheDocument();
  });

  it('does not contain a hard-coded abbreviated product version', () => {
    expect(hostSidebarSource).not.toContain('v0.1');
  });
});
