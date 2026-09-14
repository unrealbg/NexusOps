import { useQuery } from '@tanstack/react-query';
import { hostApi } from './client';

export const hostKeys = {
  all: ['hosts'] as const,
  session: (hostId: string) => ['sessions', hostId] as const,
};

export function useHosts() {
  return useQuery({
    queryKey: hostKeys.all,
    queryFn: hostApi.list,
    staleTime: 30_000,
    retry: false,
  });
}

export function useHostSession(hostId: string, interval = 1_000) {
  return useQuery({
    queryKey: hostKeys.session(hostId),
    queryFn: () => hostApi.session(hostId),
    refetchInterval: interval,
    retry: false,
  });
}
