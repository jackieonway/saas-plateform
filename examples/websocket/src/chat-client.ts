import { io, type Socket } from 'socket.io-client';

export type User = { id: string; username: string };
export type Message = {
  id: string;
  username: string;
  content: string;
  timestamp: string | Date;
  type: 'user' | 'system';
};

type ServerEvents = {
  message: (message: Message) => void;
  'user-joined': (data: { user: User; message: Message }) => void;
  'user-left': (data: { user: User; message: Message }) => void;
  'users-list': (data: { users: User[] }) => void;
};
type ClientEvents = {
  join: (data: { username: string }) => void;
  message: (data: { content: string; username: string }) => void;
};

export type ChatSocket = Socket<ServerEvents, ClientEvents>;

export function createChatSocket(endpoint: string): ChatSocket {
  return io(endpoint, {
    // The Rust chat reference uses '/', unlike the platform's '/socket.io' notifications.
    path: '/',
    autoConnect: false,
    withCredentials: false,
    transports: ['websocket', 'polling'],
    forceNew: true,
    reconnection: true,
    reconnectionAttempts: 5,
    reconnectionDelay: 1000,
    timeout: 10000,
  });
}
