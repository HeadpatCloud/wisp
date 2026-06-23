import { render, screen } from '@testing-library/react'
import { expect, test } from 'vitest'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from './resizable'

test('renders panels and a drag handle', () => {
  render(
    <ResizablePanelGroup orientation="horizontal">
      <ResizablePanel defaultSize="50%">left</ResizablePanel>
      <ResizableHandle withHandle />
      <ResizablePanel defaultSize="50%">right</ResizablePanel>
    </ResizablePanelGroup>,
  )
  expect(screen.getByText('left')).toBeInTheDocument()
  expect(screen.getByText('right')).toBeInTheDocument()
})
