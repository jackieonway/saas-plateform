import { createRoot } from 'react-dom/client';
import SocketDemo from '../frontend';
import './styles.css';

const container = document.getElementById('root');
if (!container) throw new Error('The demo root element is missing');
createRoot(container).render(<SocketDemo />);
