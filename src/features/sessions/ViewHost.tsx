import { GroupPage } from '@/features/profiles/GroupPage'
import { ImportPage } from '@/features/profiles/ImportPage'
import { ProfilePage } from '@/features/profiles/ProfilePage'
import { SettingsPage } from '@/features/settings/SettingsPage'
import { WelcomePage } from '@/features/welcome/WelcomePage'
import type { ViewTab } from '@/stores/sessionStore'

export function ViewHost({ tab }: { tab: ViewTab }) {
  switch (tab.view.kind) {
    case 'welcome':
      return <WelcomePage />
    case 'profile-editor':
      return <ProfilePage profileId={tab.view.profileId} tabId={tab.id} />
    case 'group-editor':
      return <GroupPage groupId={tab.view.groupId} tabId={tab.id} />
    case 'settings':
      return <SettingsPage tabId={tab.id} />
    case 'import':
      return <ImportPage tabId={tab.id} />
    default:
      return null
  }
}
