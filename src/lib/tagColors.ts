// Spelled out in full so Tailwind finds every class.
export const TAG_COLORS: {
  name: string
  label: string
  dot: string
  ring: string
  stripe: string
}[] = [
  { name: 'red', label: 'Red', dot: 'bg-tag-red', ring: 'ring-tag-red', stripe: 'border-tag-red' },
  {
    name: 'orange',
    label: 'Orange',
    dot: 'bg-tag-orange',
    ring: 'ring-tag-orange',
    stripe: 'border-tag-orange',
  },
  {
    name: 'amber',
    label: 'Amber',
    dot: 'bg-tag-amber',
    ring: 'ring-tag-amber',
    stripe: 'border-tag-amber',
  },
  {
    name: 'green',
    label: 'Green',
    dot: 'bg-tag-green',
    ring: 'ring-tag-green',
    stripe: 'border-tag-green',
  },
  {
    name: 'teal',
    label: 'Teal',
    dot: 'bg-tag-teal',
    ring: 'ring-tag-teal',
    stripe: 'border-tag-teal',
  },
  {
    name: 'blue',
    label: 'Blue',
    dot: 'bg-tag-blue',
    ring: 'ring-tag-blue',
    stripe: 'border-tag-blue',
  },
  {
    name: 'purple',
    label: 'Purple',
    dot: 'bg-tag-purple',
    ring: 'ring-tag-purple',
    stripe: 'border-tag-purple',
  },
  {
    name: 'pink',
    label: 'Pink',
    dot: 'bg-tag-pink',
    ring: 'ring-tag-pink',
    stripe: 'border-tag-pink',
  },
  {
    name: 'grey',
    label: 'Grey',
    dot: 'bg-tag-grey',
    ring: 'ring-tag-grey',
    stripe: 'border-tag-grey',
  },
]

export const tagColor = (name: string | null | undefined) => TAG_COLORS.find((c) => c.name === name)
