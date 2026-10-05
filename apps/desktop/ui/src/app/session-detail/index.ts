/**
 * This folder's own surface. Deliberately local: nothing here is added to the shared kit
 * barrel. The transcript is one screen's way of drawing one session's own text, reached
 * through this folder's route and its adapter; the rest of the app has no reason to render
 * a transcript out of context, and the shared kit is where it would learn to.
 */
export { SessionTranscript, TRANSCRIPT_TEXT } from './SessionTranscript';
export { TranscriptRecordView } from './TranscriptRecordView';
export type {
  CodeBlock,
  TextBlock,
  ThinkingBlock,
  ToolCallBlock,
  ToolOutcome,
  ToolResultBlock,
  TranscriptBlock,
  TranscriptPayload,
  TranscriptRecord,
  TranscriptRole,
  TranscriptState,
  TranscriptTime,
  UnsupportedBlock,
} from './transcript-view';
export { SessionDetailPage } from './SessionDetailPage';
export { blockId, transcriptProps, type TranscriptProps } from './transcript-adapter';
