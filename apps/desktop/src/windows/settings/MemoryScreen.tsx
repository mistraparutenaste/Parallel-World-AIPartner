import { MemoryDeletionPanel } from './MemoryDeletionPanel';
import { MemoryTransferPanel } from './MemoryTransferPanel';
import { SavedMemoriesPanel } from './SavedMemoriesPanel';
import { SecretModePanel } from './SecretModePanel';
import { SelfReviewPanel } from './SelfReviewPanel';
import { TaskPanel } from './TaskPanel';

export function MemoryScreen() {
  return (
    <div className="panel-stack memory-screen">
      <SelfReviewPanel />
      <SecretModePanel />
      <TaskPanel />
      <SavedMemoriesPanel />
      <MemoryTransferPanel />
      <MemoryDeletionPanel />
    </div>
  );
}
