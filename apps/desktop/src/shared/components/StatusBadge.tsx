import type { ConversationStateDto } from '@parallel-world/contracts';

const STATE_LABELS: Record<ConversationStateDto, string> = {
  starting: '準備中',
  idle: '待機中',
  listening: '聞き取り中',
  transcribing: '認識中',
  thinking: '思考中',
  speaking: '発話中',
  muted: 'ミュート中',
  interrupting: '割り込み中',
  cancelled: 'キャンセル済み',
  recovering: '復旧中',
  stt_unavailable: '音声認識が利用できません',
  llm_unavailable: '言語モデルが利用できません',
  tts_unavailable: '音声合成が利用できません',
  renderer_unavailable: 'キャラクター表示が利用できません',
};

type StatusBadgeProps = {
  state: ConversationStateDto;
};

export function StatusBadge({ state }: StatusBadgeProps) {
  return <output role="status">{STATE_LABELS[state]}</output>;
}
