/**
 * My notes: the folder Chief of Staff keeps for the user (`services::notes`).
 * The core watches the folder, so a note saved here, written by an agent or
 * edited in another editor arrives as `live://changed`, which refetches
 * exactly that note and the list (`lib/live-changes.ts`).
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'

export const notesKeys = {
  all: ['notes'] as const,
  list: ['notes', 'list'] as const,
  note: (name: string) => ['notes', 'note', name] as const,
  suggestions: ['notes', 'suggestions'] as const,
}

export function useMyNotes() {
  return useQuery({
    queryKey: notesKeys.list,
    queryFn: async () => unwrapResult(await client.getMyNotes()),
  })
}

export function useNote(name: string | null) {
  return useQuery({
    queryKey: notesKeys.note(name ?? ''),
    queryFn: async () => unwrapResult(await client.readNote(name ?? '')),
    enabled: name !== null,
  })
}

export function useSaveNote() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({ name, content }: { name: string; content: string }) =>
      unwrapResult(await client.saveNote(name, content)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: notesKeys.all }),
  })
}

/** The Chief of Staff's open suggestions (suggestions.md `- [ ]` lines) */
export function useNoteSuggestions() {
  return useQuery({
    queryKey: notesKeys.suggestions,
    queryFn: async () => unwrapResult(await client.listNoteSuggestions()),
  })
}

/** Accept (tick) or dismiss (remove) one; resolves to its text */
export function useAnswerSuggestion() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({ id, accept }: { id: string; accept: boolean }) =>
      unwrapResult(await client.answerNoteSuggestion(id, accept)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: notesKeys.all }),
  })
}

export function useDeleteNote() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (name: string) =>
      unwrapResult(await client.deleteNote(name)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: notesKeys.all }),
  })
}

/** Lets Chief of Staff keep the notes (no asking per write), or not */
export function useSetNotesAgentsCanWrite() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (allow: boolean) =>
      unwrapResult(await client.setNotesAgentsCanWrite(allow)),
    onSuccess: notes => queryClient.setQueryData(notesKeys.list, notes),
  })
}
