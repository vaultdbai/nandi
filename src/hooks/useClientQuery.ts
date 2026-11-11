import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryKey,
  type UseMutationOptions,
  type UseQueryOptions,
} from '@tanstack/react-query'
import { unwrapResult, type Result } from '@/lib/tauri-bindings'

/** What every core call answers with */
type CoreCall<Args extends unknown[], T> = (
  ...args: Args
) => Promise<Result<T, string>>

/**
 * A query over a core call. The call's error becomes the query's `Error`
 * (see `unwrapResult`), so a view reads `error.message`.
 */
export function useClientQuery<T>(
  queryKey: QueryKey,
  call: CoreCall<[], T>,
  options?: Omit<UseQueryOptions<T, Error>, 'queryKey' | 'queryFn'>
) {
  return useQuery<T, Error>({
    ...options,
    queryKey,
    queryFn: async () => unwrapResult(await call()),
  })
}

type ClientMutationOptions<T, Vars> = Omit<
  UseMutationOptions<T, Error, Vars>,
  'mutationFn'
> & {
  /** Refetched when the call ends, whether it worked or not */
  invalidates?: QueryKey
}

/** A mutation over a core call, with the same error handling */
export function useClientMutation<T, Vars = void>(
  call: CoreCall<[Vars], T>,
  { invalidates, ...options }: ClientMutationOptions<T, Vars> = {}
) {
  const queryClient = useQueryClient()
  return useMutation<T, Error, Vars>({
    ...options,
    mutationFn: async vars => unwrapResult(await call(vars)),
    // Awaited, so the mutation stays pending until the lists are fresh
    onSettled: async (...args) => {
      if (invalidates) {
        await queryClient.invalidateQueries({ queryKey: invalidates })
      }
      await options.onSettled?.(...args)
    },
  })
}
