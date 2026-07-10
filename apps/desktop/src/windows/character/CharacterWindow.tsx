import { StatusBadge } from '../../shared/components/StatusBadge';
import '../../shared/styles/global.css';

export function CharacterWindow() {
  return (
    <main className="character-stage">
      <div className="character-stage__drag" aria-hidden="true">
        {Array.from({ length: 9 }, (_, index) => <i key={index} />)}
      </div>
      <div className="character-stage__status" role="status">
        <StatusBadge>準備中</StatusBadge>
      </div>
    </main>
  );
}
