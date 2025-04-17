import express from 'express';
import path from 'path';
import { fileURLToPath } from 'url';

const app = express();
const port = 3000;

// Get directory name in ES modules (still potentially useful for other things)
// const __filename = fileURLToPath(import.meta.url);
// const __dirname = path.dirname(__filename);

// Middleware to parse JSON requests
app.use(express.json());

// Add CORS headers
app.use((req, res, next) => {
    res.header('Access-Control-Allow-Origin', '*');
    res.header('Access-Control-Allow-Headers', 'Origin, X-Requested-With, Content-Type, Accept');
    res.header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    next();
});

// Store current video state
let videoState = {
    currentTime: 0,
    isPlaying: false,
    duration: 0,
    videoPath: ''
};

// Endpoint to get current video state
app.get('/state', (req, res) => {
    res.json(videoState);
});

// Endpoint to update video state
app.post('/sync', (req, res) => {
    videoState = { ...videoState, ...req.body };
    console.log('Video state updated:', videoState);
    res.json({ success: true });
});

// Start the server (no longer needs async init)
app.listen(port, () => {
    console.log(`Node.js helper server (API only) running on http://localhost:${port}`);
    // Removed console.log for templates directory
}); 