<script>
	import '/src/app.css';
	import { onMount } from 'svelte';

	onMount(() => {
		if (typeof window !== 'undefined') {
			window.addEventListener('unhandledrejection', (event) => {
				const reason = event.reason?.message || String(event.reason || '');
				if (reason.includes('MetaMask') || reason.includes('chrome-extension://') || reason.includes('nkbihfbeogaeaoehlefnkodbefgpgknn')) {
					event.preventDefault();
					event.stopPropagation();
				}
			});
			window.addEventListener('error', (event) => {
				const filename = event.filename || '';
				const message = event.message || '';
				if (filename.includes('chrome-extension://') || message.includes('MetaMask') || message.includes('nkbihfbeogaeaoehlefnkodbefgpgknn')) {
					event.preventDefault();
					event.stopPropagation();
				}
			});
		}
	});
</script>

<slot />
