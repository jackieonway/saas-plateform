import { useEffect, useState } from 'react';
import { createChatSocket, type ChatSocket, type Message, type User } from './src/chat-client';

const endpoint = import.meta.env.VITE_CHAT_ENDPOINT as string;

export default function SocketDemo() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [inputMessage, setInputMessage] = useState('');
  const [username, setUsername] = useState('');
  const [isUsernameSet, setIsUsernameSet] = useState(false);
  const [socket, setSocket] = useState<ChatSocket | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [users, setUsers] = useState<User[]>([]);
  const [connectionError, setConnectionError] = useState(false);

  useEffect(() => {
    if (!endpoint) return;
    const socketInstance = createChatSocket(endpoint);

    setSocket(socketInstance);

    socketInstance.on('connect', () => {
      setIsConnected(true);
      setConnectionError(false);
    });

    socketInstance.on('connect_error', () => setConnectionError(true));

    socketInstance.on('disconnect', () => {
      setIsConnected(false);
      // Reconnecting creates a new server-side membership; require joining again.
      setIsUsernameSet(false);
      setUsers([]);
    });

    socketInstance.on('message', (msg: Message) => {
      setMessages(prev => [...prev, msg]);
    });

    socketInstance.on('user-joined', (data: { user: User; message: Message }) => {
      setMessages(prev => [...prev, data.message]);
      setUsers(prev => {
        if (!prev.find(u => u.id === data.user.id)) {
          return [...prev, data.user];
        }
        return prev;
      });
    });

    socketInstance.on('user-left', (data: { user: User; message: Message }) => {
      setMessages(prev => [...prev, data.message]);
      setUsers(prev => prev.filter(u => u.id !== data.user.id));
    });

    socketInstance.on('users-list', (data: { users: User[] }) => {
      setUsers(data.users);
    });

    return () => {
      socketInstance.removeAllListeners();
      socketInstance.disconnect();
    };
  }, []);

  const handleJoin = () => {
    if (socket?.connected && username.trim()) {
      socket.emit('join', { username: username.trim() });
      setIsUsernameSet(true);
    }
  };

  const sendMessage = () => {
    if (socket?.connected && isUsernameSet && inputMessage.trim() && username.trim()) {
      socket.emit('message', {
        content: inputMessage.trim(),
        username: username.trim()
      });
      setInputMessage('');
    }
  };

  return (
    <main>
      <section className="card">
        <header className="card-header">
          <h1>WebSocket Demo <span className={`status${isConnected ? ' connected' : ''}`} role="status">{isConnected ? 'Connected' : 'Disconnected'}</span></h1>
        </header>
        <div className="card-content">
          <p className="notice">Standalone chat protocol demo. This is separate from platform notifications.</p>
          {!endpoint ? (
            <p className="notice">Offline: configure the public VITE_CHAT_ENDPOINT origin and rebuild to enable a controlled chat service.</p>
          ) : (
            <button type="button" onClick={() => isConnected ? socket?.disconnect() : socket?.connect()} disabled={!socket}>{isConnected ? 'Disconnect' : 'Connect'}</button>
          )}
          {connectionError && <p className="error" role="alert">Cannot connect to the configured chat service.</p>}
          {!isUsernameSet ? (
            <form className="join" onSubmit={(event) => { event.preventDefault(); handleJoin(); }}>
              <input
                aria-label="Username"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                placeholder="Enter your username..."
                disabled={!isConnected}
              />
              <button type="submit" disabled={!isConnected || !username.trim()}>Join Chat</button>
            </form>
          ) : (
            <>
              <p className="notice">Online users: {users.length}</p>
              <div className="messages" role="log" aria-label="Chat messages" aria-live="polite">
                {messages.length === 0 ? <p className="empty">No messages yet</p> : messages.map((message) => (
                  <div key={message.id} className={`message ${message.type}`}>
                    <div className="message-body"><p className="username">{message.username}</p><p className="content">{message.content}</p></div>
                    <time>{new Date(message.timestamp).toLocaleTimeString()}</time>
                  </div>
                ))}
              </div>
              <form className="composer" onSubmit={(event) => { event.preventDefault(); sendMessage(); }}>
                <input
                  aria-label="Message"
                  value={inputMessage}
                  onChange={(e) => setInputMessage(e.target.value)}
                  placeholder="Type a message..."
                  disabled={!isConnected}
                />
                <button type="submit" disabled={!isConnected || !inputMessage.trim()}>Send</button>
              </form>
            </>
          )}
        </div>
      </section>
    </main>
  );
}
