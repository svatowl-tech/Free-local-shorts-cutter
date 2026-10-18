<script lang="ts">
	import '../app.css';

	let { children } = $props();
</script>

<div class="min-h-screen bg-[#0c0c0e] text-[#e2e2e4] antialiased">
	{@render children?.()}
</div>
