<script lang="ts">
  import KanbanBoardPage from '$lib/components/pages/KanbanBoardPage.svelte';
  import type { PageNode } from '$lib/raisin';

  interface Props {
    data: {
      board: PageNode;
      boardId: string;
    };
  }

  let { data }: Props = $props();
</script>

<KanbanBoardPage page={data.board} boardId={data.boardId} />
