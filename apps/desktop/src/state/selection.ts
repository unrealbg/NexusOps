import { create } from 'zustand';

// Only ephemeral UI selection belongs here. Hosts, sessions and credentials never do.
export const useSelection = create<{
  hostId: string | null;
  select: (hostId: string | null) => void;
}>((set) => ({
  hostId: null,
  select: (hostId) => set({ hostId }),
}));
