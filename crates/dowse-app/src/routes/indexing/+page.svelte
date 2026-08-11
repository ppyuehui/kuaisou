<script lang="ts">
	// fork 新增：独立的"索引进度窗口"页面（窗口 label="indexing"）。
	// 建索引是后台线程，主窗口不再卡死；这个窗口负责把进度可视化——文本阶段
	// 显示"已处理数 + 当前文件"，OCR 阶段显示带比例的进度条。窗口可最小化，
	// 任务栏按钮上的进度由 Rust 侧（indexing_status.rs::sync_window）用
	// set_progress_bar 驱动，这里只管窗口内部的内容。
	//
	// 数据来源跟主窗口完全一致：挂载时拉一次快照（indexing_status 命令），
	// 之后靠既有事件流续播（rebuild-progress / ocr-progress / indexing-settled /
	// rebuild-done / rebuild-error）。窗口是启动时创建、隐藏、需要时 show 的，
	// 页面在隐藏期间也一直在收事件，show 出来那一刻内容就是最新的。

	import { onMount } from 'svelte';
	import { listen } from '@tauri-apps/api/event';
	import * as api from '$lib/api';
	import type { IndexProgress, IndexingSnapshot } from '$lib/types';

	let snapshot = $state<IndexingSnapshot>({
		phase: 'idle',
		text_processed: 0,
		text_current_file: '',
		ocr_processed: 0,
		ocr_total: 0
	});
	let doneText = $state('');
	let errorText = $state('');

	function applySnap(next: IndexingSnapshot) {
		snapshot = next;
	}

	const PHASE_LABEL: Record<string, string> = {
		text: '正在建索引',
		ocr: '图片识别'
	};

	onMount(() => {
		api.indexingStatus().then(applySnap).catch(() => {});

		const unlisten = [
			listen<IndexProgress>('dowse://rebuild-progress', (evt) => {
				doneText = '';
				errorText = '';
				snapshot = {
					phase: 'text',
					text_processed: evt.payload.processed,
					text_current_file: evt.payload.path,
					ocr_processed: snapshot.ocr_processed,
					ocr_total: snapshot.ocr_total
				};
			}),
			listen<number>('dowse://ocr-progress', (evt) => {
				const pending = evt.payload;
				snapshot = {
					phase: pending > 0 ? 'ocr' : 'idle',
					text_processed: snapshot.text_processed,
					text_current_file: snapshot.text_current_file,
					ocr_processed: pending > 0 ? Math.max(snapshot.ocr_total - pending, 0) : snapshot.ocr_processed,
					ocr_total: pending > snapshot.ocr_total ? pending : snapshot.ocr_total
				};
			}),
			listen<IndexingSnapshot>('dowse://indexing-settled', (evt) => {
				applySnap(evt.payload);
			}),
			listen<number>('dowse://rebuild-done', (evt) => {
				doneText = `完成 · 收录 ${evt.payload} 篇`;
			}),
			listen<string>('dowse://rebuild-error', (evt) => {
				errorText = `失败：${evt.payload}`;
			})
		];

		return () => {
			unlisten.forEach((u) => u.then((f) => f()));
		};
	});

	let ocrPercent = $derived(
		snapshot.phase === 'ocr' && snapshot.ocr_total > 0
			? Math.round((snapshot.ocr_processed / snapshot.ocr_total) * 100)
			: 0
	);
</script>

<div class="win">
	<div class="head">
		<span class="dot" class:active={snapshot.phase !== 'idle'}></span>
		<span class="title">dowse</span>
		<span class="phase">{snapshot.phase !== 'idle' ? (PHASE_LABEL[snapshot.phase] ?? '') : ''}</span>
	</div>

	{#if errorText}
		<div class="status error">{errorText}</div>
	{:else if doneText}
		<div class="status ok">{doneText}</div>
	{:else if snapshot.phase === 'ocr'}
		<div class="bar"><div class="fill" style={`width: ${ocrPercent}%`}></div></div>
		<div class="meta">
			<span>图片识别 {snapshot.ocr_processed} / {snapshot.ocr_total} · {ocrPercent}%</span>
		</div>
	{:else if snapshot.phase === 'text'}
		<div class="bar indeterminate"></div>
		<div class="meta">
			<span>已处理 {snapshot.text_processed} 个文件</span>
			<span class="file" title={snapshot.text_current_file}>{snapshot.text_current_file || '…'}</span>
		</div>
	{:else}
		<div class="status idle">空闲</div>
	{/if}
</div>

<style>
	.win {
		display: flex;
		flex-direction: column;
		gap: 14px;
		padding: 16px 18px;
		box-sizing: border-box;
		height: 100vh;
		font-family: 'Segoe UI', 'Microsoft YaHei', system-ui, sans-serif;
		background: #f3f3f3;
		color: #1f2328;
	}

	.head {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: #b0b7c0;
	}

	.dot.active {
		background: #1f8a4c;
		animation: pulse 1.2s ease-in-out infinite;
	}

	@keyframes pulse {
		0%,
		100% {
			opacity: 1;
		}
		50% {
			opacity: 0.4;
		}
	}

	.title {
		font-size: 13px;
		font-weight: 600;
	}

	.phase {
		margin-left: auto;
		font-size: 12px;
		color: #57606a;
	}

	.bar {
		width: 100%;
		height: 8px;
		border-radius: 4px;
		background: #d8dee4;
		overflow: hidden;
	}

	.fill {
		height: 100%;
		background: #1f8a4c;
		border-radius: 4px;
		transition: width 0.15s ease-out;
	}

	.bar.indeterminate {
		position: relative;
	}

	.bar.indeterminate::after {
		content: '';
		position: absolute;
		top: 0;
		left: -40%;
		width: 40%;
		height: 100%;
		background: #1f8a4c;
		border-radius: 4px;
		animation: slide 1.1s ease-in-out infinite;
	}

	@keyframes slide {
		0% {
			left: -40%;
		}
		100% {
			left: 100%;
		}
	}

	.meta {
		display: flex;
		flex-direction: column;
		gap: 4px;
		font-size: 12px;
		color: #57606a;
	}

	.file {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		direction: rtl;
		text-align: left;
		font-size: 11px;
		color: #8b949e;
	}

	.status {
		font-size: 13px;
		padding: 8px 0;
	}

	.status.idle {
		color: #8b949e;
	}

	.status.ok {
		color: #1f8a4c;
	}

	.status.error {
		color: #cf222e;
	}
</style>
