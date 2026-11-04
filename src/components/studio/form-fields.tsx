import { useId } from 'react'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Switch } from '@/components/ui/switch'
import { getField, setField } from '@/lib/yaml-fields'

/** Every form edits the raw file through these */
export interface FormProps {
  text: string
  onChange: (text: string) => void
}

interface FieldProps extends FormProps {
  path: string
  label: string
}

function Field({
  label,
  id,
  children,
}: {
  label: string
  id: string
  children: React.ReactNode
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <Label htmlFor={id} className="text-xs text-muted-foreground">
        {label}
      </Label>
      {children}
    </div>
  )
}

export function TextField({ text, onChange, path, label }: FieldProps) {
  const id = useId()
  const value = getField(text, path)
  return (
    <Field label={label} id={id}>
      <Input
        id={id}
        value={
          typeof value === 'string' || typeof value === 'number'
            ? String(value)
            : ''
        }
        onChange={event => onChange(setField(text, path, event.target.value))}
      />
    </Field>
  )
}

export function NumberField({ text, onChange, path, label }: FieldProps) {
  const id = useId()
  const value = getField(text, path)
  return (
    <Field label={label} id={id}>
      <Input
        id={id}
        type="number"
        value={typeof value === 'number' ? value : ''}
        onChange={event => {
          const raw = event.target.value
          onChange(setField(text, path, raw === '' ? null : Number(raw)))
        }}
      />
    </Field>
  )
}

export function SwitchField({ text, onChange, path, label }: FieldProps) {
  const id = useId()
  return (
    <div className="flex items-center gap-2">
      <Switch
        id={id}
        checked={getField(text, path) === true}
        onCheckedChange={checked => onChange(setField(text, path, checked))}
      />
      <Label htmlFor={id} className="text-sm">
        {label}
      </Label>
    </div>
  )
}

/** A comma-separated list, written as `[a, b]` */
export function ListField({ text, onChange, path, label }: FieldProps) {
  const id = useId()
  const value = getField(text, path)
  const items = Array.isArray(value) ? value : []
  return (
    <Field label={label} id={id}>
      <Input
        id={id}
        value={items.join(', ')}
        onChange={event =>
          onChange(
            setField(
              text,
              path,
              event.target.value
                .split(',')
                .map(item => item.trim())
                .filter(Boolean)
            )
          )
        }
      />
    </Field>
  )
}

export function SelectField({
  text,
  onChange,
  path,
  label,
  options,
}: FieldProps & { options: string[] }) {
  const id = useId()
  const value = getField(text, path)
  return (
    <Field label={label} id={id}>
      <NativeSelect
        id={id}
        value={typeof value === 'string' ? value : options[0]}
        onChange={event => onChange(setField(text, path, event.target.value))}
      >
        {options.map(option => (
          <NativeSelectOption key={option} value={option}>
            {option}
          </NativeSelectOption>
        ))}
      </NativeSelect>
    </Field>
  )
}

export function FormGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid gap-3 sm:grid-cols-2">{children}</div>
}
