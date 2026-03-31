import { useRaisinClient } from '../hooks/useRaisinClient';

export default function LiveIndicator() {
  const { isConnected } = useRaisinClient();

  return (
    <div className="flex items-center space-x-2 text-sm">
      <div
        className={`w-2 h-2 rounded-full ${
          isConnected ? 'bg-green-500' : 'bg-red-500'
        } ${isConnected ? 'animate-pulse' : ''}`}
      />
      <span className="text-gray-600">
        {isConnected ? 'Live' : 'Disconnected'}
      </span>
    </div>
  );
}
