import { Routes, Route, Link } from 'react-router';
import { useAuth } from './hooks/useAuth';
import Feed from './pages/Feed';
import Profile from './pages/Profile';
import Post from './pages/Post';
import Admin from './pages/Admin';
import LiveIndicator from './components/LiveIndicator';

export default function App() {
  const { isConnected, isAuthenticated } = useAuth();

  if (!isConnected) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-100">
        <div className="text-center">
          <div className="inline-block animate-spin rounded-full h-12 w-12 border-b-2 border-blue-600"></div>
          <p className="mt-4 text-gray-600">Connecting to RaisinDB...</p>
        </div>
      </div>
    );
  }

  if (!isAuthenticated) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-100">
        <div className="text-center card">
          <h2 className="text-2xl font-bold mb-2">Authentication Required</h2>
          <p className="text-gray-600">Please check your credentials</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50">
      {/* Navigation */}
      <nav className="bg-white shadow-sm border-b">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div className="flex justify-between h-16">
            <div className="flex space-x-8">
              <Link
                to="/"
                className="inline-flex items-center px-1 pt-1 text-sm font-medium text-gray-900"
              >
                🏠 Feed
              </Link>
              <Link
                to="/admin"
                className="inline-flex items-center px-1 pt-1 text-sm font-medium text-gray-500 hover:text-gray-900"
              >
                ⚙️ Admin
              </Link>
            </div>
            <div className="flex items-center">
              <LiveIndicator />
            </div>
          </div>
        </div>
      </nav>

      {/* Main Content */}
      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <Routes>
          <Route path="/" element={<Feed />} />
          <Route path="/profile/:userId" element={<Profile />} />
          <Route path="/post/:postId" element={<Post />} />
          <Route path="/admin" element={<Admin />} />
        </Routes>
      </main>
    </div>
  );
}
