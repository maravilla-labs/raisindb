import { useHybridClient } from '~/hooks/useHybridClient';

export default function LiveIndicator() {
  const { mode, isRealtime } = useHybridClient();

  const getStatusColor = () => {
    switch (mode) {
      case 'websocket':
        return 'bg-green-500';
      case 'connecting':
        return 'bg-yellow-500';
      case 'error':
        return 'bg-red-500';
      default:
        return 'bg-gray-400';
    }
  };

  const getStatusText = () => {
    switch (mode) {
      case 'websocket':
        return 'Live (SSR)';
      case 'connecting':
        return 'Connecting...';
      case 'error':
        return 'Connection Error';
      default:
        return 'HTTP Mode';
    }
  };

  return (
    <div className="flex items-center space-x-2 text-sm">
      <div
        className={`w-2 h-2 rounded-full ${getStatusColor()} ${
          isRealtime ? 'animate-pulse' : ''
        }`}
      />
      <span className="text-gray-600 dark:text-gray-400">
        {getStatusText()}
      </span>
    </div>
  );
}
