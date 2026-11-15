/**
 * My notes: the folder Nandi keeps for the user (`services::notes`).
 * The core watches the folder, so a note saved here, written by an agent or
 * edited in another editor arrives as `live://changed`, which refetches
 * exactly that note and the list (`lib/live-changes.ts`).
 */
import { useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

export const notesKeys = {
  all: ['notes'] as const,
  list: ['notes', 'list'] as const,
  note: (name: string) => ['notes', 'note', name] as const,
  suggestions: ['notes', 'suggestions'] as const,
}

export function useMyNotes() {
  return useClientQuery(notesKeys.list, () => client.getMyNotes())
}

export function useNote(name: string | null) {
  return useClientQuery(
    notesKeys.note(name ?? ''),
    () => client.readNote(name ?? ''),
    { enabled: name !== null }
  )
}

export function useSaveNote() {
  const queryClient = useQueryClient()
  return useClientMutation(
    ({ name, content }: { name: string; content: string }) =>
      client.saveNote(name, content),
    {
      onSettled: () =>
        queryClient.invalidateQueries({ queryKey: notesKeys.all }),
    }
  )
}

/** Nandi's open suggestions (suggestions.md `- [ ]` lines) */
export function useNoteSuggestions() {
  return useClientQuery(notesKeys.suggestions, () =>
    client.listNoteSuggestions()
  )
}

/** Accept (tick) or dismiss (remove) one; resolves to its text */
export function useAnswerSuggestion() {
  const queryClient = useQueryClient()
  return useClientMutation(
    ({ id, accept }: { id: string; accept: boolean }) =>
      client.answerNoteSuggestion(id, accept),
    {
      onSettled: () =>
        queryClient.invalidateQueries({ queryKey: notesKeys.all }),
    }
  )
}

export function useDeleteNote() {
  const queryClient = useQueryClient()
  return useClientMutation((name: string) => client.deleteNote(name), {
    onSettled: () => queryClient.invalidateQueries({ queryKey: notesKeys.all }),
  })
}

/** Lets Nandi keep the notes (no asking per write), or not */
export function useSetNotesAgentsCanWrite() {
  const queryClient = useQueryClient()
  return useClientMutation(
    (allow: boolean) => client.setNotesAgentsCanWrite(allow),
    { onSuccess: notes => queryClient.setQueryData(notesKeys.list, notes) }
  )
}
